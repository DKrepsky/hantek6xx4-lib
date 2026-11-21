//! High-level scope handle: initialisation, configuration and acquisition.

use crate::error::{Error, Result};
use crate::protocol::{self as p, BufferStatus, Calibration, SpiChip, State};
use crate::transport::{Transport, UsbTransport};
use crate::types::{Config, Frame, ScopeInfo, Sweep, MAX_CHANNELS};
use std::time::{Duration, Instant};

/// Options for one acquisition.
#[derive(Debug, Clone, Copy)]
pub struct CaptureOptions {
    /// How long to wait for the capture to complete.
    pub timeout: Duration,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(2),
        }
    }
}

/// An initialised oscilloscope.
pub struct Scope<T: Transport> {
    transport: T,
    packet_size: usize,
    calibration: Calibration,
    info: ScopeInfo,
    pcb_revision: u32,
    config: Config,
}

impl Scope<UsbTransport> {
    /// Open and initialise the first connected scope.
    pub fn open() -> Result<Self> {
        Self::new(UsbTransport::open()?)
    }

    /// Open and initialise the `n`-th connected scope.
    pub fn open_nth(n: usize) -> Result<Self> {
        Self::new(UsbTransport::open_nth(n)?)
    }
}

impl<T: Transport> Scope<T> {
    /// Initialise a scope on an already opened transport: reset, read identification and calibration, bring up ADC and PLL.
    pub fn new(mut transport: T) -> Result<Self> {
        let mut packet_size = 512;
        Self::send_raw(&mut transport, &mut packet_size, &p::CMD_RESET)?;
        let reply = transport.bulk_in(packet_size)?;
        if reply.len() < 2 {
            return Err(Error::Protocol("reset reply too short".into()));
        }
        let fpga_version = u16::from_le_bytes([reply[0], reply[1]]);

        let raw_info = transport.control_in(p::REQ_AREA, p::AREA_INFO, p::INFO_LEN)?;
        let (name, pcb_version, serial) = p::parse_info(&raw_info);
        let pcb_revision = p::parse_pcb_revision(raw_info.get(14..20).unwrap_or(&[]));
        let cal_bytes = transport.control_in(p::REQ_AREA, p::AREA_CALIBRATION, p::CAL_WORDS * 2)?;
        let calibration = Calibration::from_bytes(&cal_bytes)?;

        for (chip, reg, value, sleep) in p::INIT_SEQUENCE {
            Self::send_raw(&mut transport, &mut packet_size, &p::spi_write(chip, reg, value))?;
            transport.delay(sleep);
        }
        log::info!("initialised scope: name={name} pcb={pcb_version} fpga=0x{fpga_version:04x}");
        Ok(Self {
            transport,
            packet_size,
            calibration,
            info: ScopeInfo {
                raw_info,
                name,
                pcb_version,
                serial,
                fpga_version,
            },
            pcb_revision,
            config: Config::default(),
        })
    }

    /// Identification data.
    pub fn info(&self) -> &ScopeInfo {
        &self.info
    }

    /// The calibration table read from the device.
    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    /// The configuration last applied with [`Scope::configure`].
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Access the transport (for tests).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Mutable access to the transport (low level, for experiments).
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    // ---- low level -----------------------------------------------------------------------

    fn send_raw(transport: &mut T, packet_size: &mut usize, data: &[u8]) -> Result<()> {
        transport.control_out(p::REQ_PREAMBLE, 0, &p::preamble())?;
        let ready = transport.control_in(p::REQ_READY, 0, 10)?;
        *packet_size = if ready.first().copied().unwrap_or(0) != 0 {
            512
        } else {
            64
        };
        transport.bulk_out(data)
    }

    /// Send a raw bulk command (low level, for experiments). Preceded by the mandatory control preamble.
    pub fn send(&mut self, data: &[u8]) -> Result<()> {
        Self::send_raw(&mut self.transport, &mut self.packet_size, data)
    }

    /// Read a raw bulk reply of up to `len` bytes (low level).
    pub fn recv(&mut self, len: usize) -> Result<Vec<u8>> {
        self.transport.bulk_in(len)
    }

    /// Send a command and read one packet back.
    pub fn query(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.send(data)?;
        let n = self.packet_size;
        self.recv(n)
    }

    fn spi(&mut self, chip: SpiChip, reg: u8, value: u16) -> Result<()> {
        self.send(&p::spi_write(chip, reg, value))
    }

    fn write_adc_registers(&mut self, cfg: &Config) -> Result<()> {
        let vdiv = cfg.channels.map(|c| c.volts_per_div.index());
        for (reg, value) in p::adc_register_writes(cfg.channel_mask(), vdiv, cfg.timebase) {
            self.spi(SpiChip::Adc, reg, value)?;
            self.transport.delay(3);
        }
        Ok(())
    }

    // ---- configuration -------------------------------------------------------------------

    /// Apply a complete configuration, in the order used by the vendor software.
    pub fn configure(&mut self, cfg: &Config) -> Result<()> {
        cfg.validate()?;
        let mask = cfg.channel_mask();
        let n = cfg.enabled_count();
        let k = p::channel_mode(n, cfg.timebase);
        let interval = p::sample_interval_ns(cfg.timebase, k);

        self.spi(SpiChip::Adc, 0x55, p::adc_gain_register(self.pcb_revision, k))?;
        // sample rate
        self.write_adc_registers(cfg)?;
        self.send(&p::cmd_timebase(cfg.timebase))?;
        let h = p::cmd_horizontal(
            interval,
            cfg.buffer_len,
            cfg.trigger.horizontal_position_percent,
            self.info.fpga_version,
        );
        self.send(&h)?;
        // channels
        let (first, second) = p::cmd_relays(&cfg.channels);
        self.send(&first)?;
        self.transport.delay(4);
        self.send(&second)?;
        self.transport.delay(50);
        self.write_adc_registers(cfg)?;
        // trigger source
        self.send(&p::cmd_trigger_source(
            cfg.timebase,
            mask,
            cfg.trigger.source.index(),
            false,
        ))?;
        // positions
        for (ch, c) in cfg.channels.iter().enumerate().take(MAX_CHANNELS) {
            let word = self
                .calibration
                .position_word(ch, c.volts_per_div.index(), k, c.position);
            self.send(&p::cmd_channel_position(ch, word))?;
            self.transport.delay(10);
        }
        self.send(&p::cmd_trigger_level(cfg.trigger.level))?;
        self.send(&p::cmd_trigger_mode_edge(cfg.trigger.slope))?;
        self.config = cfg.clone();
        Ok(())
    }

    // ---- acquisition ---------------------------------------------------------------------

    /// Run one acquisition with the current configuration.
    ///
    /// Triggered captures return the window aligned to the trigger position. In auto mode without a trigger
    /// the first samples from the start of the capture RAM are returned (their first few frames may be stale).
    pub fn capture(&mut self, opts: CaptureOptions) -> Result<Frame> {
        let cfg = self.config.clone();
        let n = cfg.enabled_count();
        if n == 0 {
            return Err(Error::invalid("scope has not been configured"));
        }
        let stride = p::frame_stride(n);

        self.send(&p::cmd_start(cfg.trigger.sweep))?;
        let deadline = Instant::now() + opts.timeout;
        let state = loop {
            let reply = self.query(&p::CMD_STATE)?;
            let state = State::parse(&reply)?;
            if state.data_ready() {
                break state;
            }
            if Instant::now() >= deadline {
                let _ = self.send(&p::CMD_STOP);
                return Err(Error::Timeout("capture to complete"));
            }
            self.transport.delay(20);
        };

        let status = BufferStatus::parse(&self.query(&p::CMD_BUFFER_STATUS)?)?;
        let bytes = cfg.buffer_len as usize * stride;
        let start = if state.triggered() {
            (status.end_address as usize).wrapping_sub(bytes) & 0xffff & !7
        } else {
            0
        };
        self.send(&p::cmd_set_read_address(start as u16))?;
        self.send(&p::cmd_readback(bytes as u32))?;
        let mut data = Vec::with_capacity(bytes);
        while data.len() < bytes {
            let chunk = self.recv(bytes - data.len())?;
            if chunk.is_empty() {
                return Err(Error::Protocol(format!(
                    "readback ended after {} of {bytes} bytes",
                    data.len()
                )));
            }
            data.extend_from_slice(&chunk);
        }

        let skip = if state.triggered() {
            0
        } else {
            16.min(cfg.buffer_len as usize)
        };
        let mut samples: [Vec<u8>; MAX_CHANNELS] = Default::default();
        let enabled: Vec<usize> = (0..MAX_CHANNELS).filter(|&i| cfg.channels[i].enabled).collect();
        for (j, &ch) in enabled.iter().enumerate() {
            // 3 or 4 enabled channels use the fixed 4-channel layout; 1 or 2 channels are packed in order
            let offset = if stride == 4 { ch } else { j };
            samples[ch] = data.iter().skip(offset).step_by(stride).skip(skip).copied().collect();
        }

        let k = p::channel_mode(n, cfg.timebase);
        let trigger_sample = state
            .triggered()
            .then(|| p::pre_trigger_samples(cfg.buffer_len, cfg.trigger.horizontal_position_percent) as usize);
        Ok(Frame {
            samples,
            sample_interval_ns: p::sample_interval_ns(cfg.timebase, k),
            triggered: state.triggered(),
            trigger_sample,
        })
    }

    /// Disarm any running acquisition.
    pub fn stop(&mut self) -> Result<()> {
        self.send(&p::CMD_STOP)
    }

    /// Read the acquisition state word.
    pub fn state(&mut self) -> Result<State> {
        State::parse(&self.query(&p::CMD_STATE)?)
    }

    /// Mutable transport access used by the integration tests to script replies.
    #[doc(hidden)]
    pub fn transport_mut_for_test(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Hand back the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

/// Use [`Sweep`] in doc links.
#[allow(dead_code)]
fn _sweep_doc(_: Sweep) {}
