//! Wire format of the 6xx4 series: pure functions that turn settings into command bytes and parse replies.
//!
//! Nothing in this module performs I/O. Every function is covered by golden-byte tests taken from
//! behaviour verified on a real 6254BC (see `docs/protocol.md`).

use crate::error::{Error, Result};
use crate::types::{
    ChannelConfig, Coupling, PulseCondition, Slope, Sweep, TimeBase, TriggerKind, VideoStandard, VideoSync,
};

// ---- control requests -------------------------------------------------------------------------

/// Control request that selects the transfer mode; sent before every bulk write.
pub const REQ_PREAMBLE: u8 = 0xB3;
/// Control request that returns the ready / high-speed flag.
pub const REQ_READY: u8 = 0xB2;
/// Control request for the device memory areas (info string, calibration, ...).
pub const REQ_AREA: u8 = 0xA2;
/// wValue of the info string area.
pub const AREA_INFO: u16 = 0x1580;
/// Length of the info string.
pub const INFO_LEN: usize = 0x47;
/// Request that reads the driver (firmware) version.
pub const REQ_DRIVER_VERSION: u8 = 0xE3;
/// wValue of the driver version.
pub const AREA_DRIVER_VERSION: u16 = 0x15F0;

/// Decode the 4 BCD bytes returned by the driver version read into its 8 digits (`"20160531"` on a 6254BC).
pub fn parse_driver_version(raw: &[u8]) -> Option<String> {
    if raw.len() < 4 {
        return None;
    }
    let mut out = String::with_capacity(8);
    for &b in &raw[..4] {
        for nibble in [b >> 4, b & 0x0f] {
            if nibble > 9 {
                return None;
            }
            out.push((b'0' + nibble) as char);
        }
    }
    Some(out)
}

/// wValue of the calibration table.
pub const AREA_CALIBRATION: u16 = 0x1600;
/// Number of 16-bit words in the calibration table including the validity flag.
pub const CAL_WORDS: usize = 577;
/// Value of the last calibration word when the table is valid.
pub const CAL_FLAG: u16 = 0xFBCF;

/// wValue of the amplitude (gain trim) table.
pub const AREA_AMPLITUDE: u16 = 0x1B00;
/// Number of 16-bit words in the amplitude table, not counting the validity flag: 4 channels x 12 rows x 6 slots.
pub const AMP_WORDS: usize = 288;
/// Value of the word after the amplitude table when it is valid.
pub const AMP_FLAG: u16 = 0xFACF;
/// Trim value meaning "no correction" (1.0 in 1/1024 units).
pub const AMP_UNITY: u16 = 0x400;

/// The 10-byte payload of the `0xB3` preamble request (mode 1).
pub fn preamble() -> [u8; 10] {
    let mut p = [0u8; 10];
    p[..4].copy_from_slice(&[0x0f, 3, 3, 3]);
    p
}

// ---- simple bulk commands ---------------------------------------------------------------------

/// Reset the capture engine; the reply carries the FPGA version in its first two bytes.
pub const CMD_RESET: [u8; 2] = [0x0c, 0x00];
/// Read the hardware version.
pub const CMD_HW_VERSION: [u8; 2] = [0x09, 0x00];
/// Read the acquisition state word.
pub const CMD_STATE: [u8; 2] = [0x06, 0x00];
/// Read the buffer status (trigger and end address).
pub const CMD_BUFFER_STATUS: [u8; 2] = [0x0d, 0x00];
/// Disarm a running acquisition.
pub const CMD_STOP: [u8; 4] = [0x03, 0x00, 0x04, 0x00];

/// Start an acquisition.
pub fn cmd_start(sweep: Sweep) -> [u8; 4] {
    [0x03, 0x00, if sweep == Sweep::Auto { 0x01 } else { 0x00 }, 0x00]
}

/// Acquisition state word (reply to [`CMD_STATE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State(pub u16);

impl State {
    /// Parse the first two bytes of a state reply.
    pub fn parse(reply: &[u8]) -> Result<Self> {
        if reply.len() < 2 {
            return Err(Error::Protocol("state reply shorter than 2 bytes".into()));
        }
        Ok(Self(u16::from_le_bytes([reply[0], reply[1]])))
    }
    /// The hardware trigger fired.
    pub fn triggered(self) -> bool {
        self.0 & 1 != 0
    }
    /// A complete capture is in memory.
    pub fn data_ready(self) -> bool {
        self.0 & 2 != 0
    }
}

/// Reply to [`CMD_BUFFER_STATUS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferStatus {
    /// Byte address of the trigger edge in the capture RAM (valid after a triggered capture).
    pub trigger_address: u16,
    /// Byte address of the end of the capture.
    pub end_address: u16,
}

impl BufferStatus {
    /// Parse the first four bytes of a buffer status reply.
    pub fn parse(reply: &[u8]) -> Result<Self> {
        if reply.len() < 4 {
            return Err(Error::Protocol("buffer status reply shorter than 4 bytes".into()));
        }
        Ok(Self {
            trigger_address: u16::from_le_bytes([reply[0], reply[1]]),
            end_address: u16::from_le_bytes([reply[2], reply[3]]),
        })
    }
}

/// Set the byte address where the next readback starts.
pub fn cmd_set_read_address(addr: u16) -> [u8; 4] {
    let b = addr.to_le_bytes();
    [0x0e, 0x00, b[0], b[1]]
}

/// Request a readback of `bytes` bytes (the device counts in 16-bit units).
pub fn cmd_readback(bytes: u32) -> [u8; 4] {
    let half = ((bytes / 2) & 0xffff) as u16;
    let b = half.to_le_bytes();
    [0x05, 0x00, b[0], b[1]]
}

/// Bytes per sample frame in the readback stream for `n` enabled channels (3 channels use the 4-channel layout).
pub fn frame_stride(enabled: usize) -> usize {
    if enabled >= 3 {
        4
    } else {
        enabled
    }
}

// ---- SPI --------------------------------------------------------------------------------------

/// SPI chip select lines routed through the FPGA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiChip {
    /// Front-end relay shift registers.
    ShiftRegisters = 1,
    /// ADF4360-7 clock synthesizer.
    Pll = 2,
    /// HMCAD1511 ADC.
    Adc = 4,
}

/// SPI write: 16-bit `value` into register `reg` of `chip`.
pub fn spi_write(chip: SpiChip, reg: u8, value: u16) -> [u8; 8] {
    let v = value.to_le_bytes();
    [0x08, 0x00, 0x00, v[0], v[1], reg, chip as u8, 0x00]
}

/// ADC and PLL bring-up: `(chip, register, value, sleep_after_ms)`.
pub const INIT_SEQUENCE: [(SpiChip, u8, u16, u64); 5] = [
    (SpiChip::Adc, 0x12, 0x4777, 2),
    (SpiChip::Adc, 0x33, 0x0003, 2),
    (SpiChip::Pll, 0x30, 0x0065, 2),
    (SpiChip::Pll, 0x0f, 0xf128, 2),
    (SpiChip::Pll, 0x01, 0x3812, 15),
];

/// Parse the PCB revision digits (e.g. `b"P1.06\xff"` gives 106), as the vendor software does.
pub fn parse_pcb_revision(field: &[u8]) -> u32 {
    let (mut sum, mut weight) = (0u32, 1u32);
    for &b in field.iter().rev() {
        if b.is_ascii_digit() {
            sum += (b - b'0') as u32 * weight;
            weight *= 10;
        }
    }
    sum
}

/// Value of ADC register `0x55` for a PCB revision and channel mode `k` (1, 2 or 4).
pub fn adc_gain_register(pcb_revision: u32, k: u8) -> u16 {
    match pcb_revision {
        105 => match k {
            4 => 0x37,
            2 => 0x0a,
            _ => 0x00,
        },
        106 => 0x3f,
        _ => match k {
            4 => 0x3f,
            2 => 0x30,
            _ => 0x19,
        },
    }
}

// ---- channel modes ----------------------------------------------------------------------------

/// ADC channel mode `k`: 1 = single (1 GS/s), 2 = dual (500 MS/s), 4 = quad (250 MS/s).
pub fn channel_mode(enabled: usize, timebase: TimeBase) -> u8 {
    let t = timebase.index();
    match enabled {
        0 => 0,
        1 => {
            if t < 7 {
                1
            } else if t == 7 {
                2
            } else {
                4
            }
        }
        2 => {
            if t > 7 {
                4
            } else {
                2
            }
        }
        _ => 4,
    }
}

/// Time between samples in nanoseconds.
pub fn sample_interval_ns(timebase: TimeBase, k: u8) -> f64 {
    if timebase.index() <= 7 {
        k as f64
    } else {
        // 250 samples per division
        timebase.ns_per_div() / 250.0
    }
}

const DIVISORS: [u32; 37] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000, 25000, 50000, 100000,
    250000, 500000, 1000000, 2500000, 5000000, 10000000, 25000000, 50000000, 100000000, 250000000, 500000000,
    500000000,
];

/// Time base command: payload is `divisor - 1`.
pub fn cmd_timebase(timebase: TimeBase) -> [u8; 6] {
    let d = (DIVISORS[timebase.index() as usize] - 1).to_le_bytes();
    [0x0f, 0x00, d[0], d[1], d[2], d[3]]
}

/// HMCAD1511 register writes for a channel mask, per-channel volts/div indices and time base.
///
/// Returns `(register, value)` in the order the vendor software writes them (3 ms apart).
pub fn adc_register_writes(mask: u8, vdiv: [u8; 4], timebase: TimeBase) -> Vec<(u8, u16)> {
    let gain = |v: u8| -> u16 {
        match v {
            0 => 0xd,
            1 => 0xa,
            2 => 0x7,
            3 | 6 | 9 => 0x5,
            4 | 7 | 10 => 0x2,
            _ => 0x0,
        }
    };
    let g: Vec<u16> = vdiv.iter().map(|&v| gain(v)).collect();
    let quad = (g[3] << 12) | (g[2] << 8) | (g[1] << 4) | g[0];
    let enabled: Vec<usize> = (0..4).filter(|i| mask >> i & 1 == 1).collect();
    let t = timebase.index();
    let (mut mode, mut r31, mut r3a, mut r3b, mut word) = (4u8, 0x0204u16, 0x0810u16, 0x0204u16, quad);
    if enabled.len() == 1 && t < 7 {
        mode = 1;
        r31 = 0x0001;
        word = g[enabled[0]] * 0x1111;
        let r = match mask {
            1 => 0x1010,
            2 => 0x0808,
            4 => 0x0404,
            _ => 0x0202,
        };
        r3a = r;
        r3b = r;
    } else if enabled.len() == 1 && t == 7 {
        mode = 2;
        r31 = 0x0102;
        word = g[enabled[0]] * 0x1111;
        (r3a, r3b) = match mask {
            1 => (0x1010, 0x0808),
            2 => (0x0404, 0x0808),
            4 => (0x0404, 0x0202),
            _ => (0x1010, 0x0202),
        };
    } else if enabled.len() == 2 && t < 8 {
        mode = 2;
        r31 = 0x0102;
        word = (g[enabled[0]] * 0x11) | ((g[enabled[1]] * 0x11) << 8);
        (r3a, r3b) = match mask {
            3 => (0x1010, 0x0808),
            5 => (0x1010, 0x0404),
            6 => (0x0808, 0x0404),
            9 => (0x1010, 0x0202),
            10 => (0x0808, 0x0202),
            _ => (0x0404, 0x0202),
        };
    }
    let mut out = vec![(0x3a, r3a), (0x3b, r3b), (0x0f, 0x0000), (0x31, r31)];
    if mode == 4 {
        out.push((0x2a, word));
    } else {
        let (lo, hi) = ((word & 0xff) as u8, (word >> 8) as u8);
        let d0 = ((hi ^ lo) & 0xf) ^ hi;
        let d1 = lo & 0xf;
        out.push((0x2b, u16::from_le_bytes([d0, d1])));
    }
    out
}

// ---- relays -----------------------------------------------------------------------------------

/// Relay shift-register messages: the full register image, then the image with the relay-drive bits cleared.
pub fn cmd_relays(channels: &[ChannelConfig; 4]) -> ([u8; 8], [u8; 8]) {
    let mut b = [0u8; 4];
    for (i, c) in channels.iter().enumerate() {
        let v = c.volts_per_div.index();
        b[i] = (c.bandwidth_limit as u8) << 7
            | ((v > 8) as u8) << 6
            | ((v <= 8) as u8) << 5
            | ((v > 5) as u8) << 4
            | ((v <= 5) as u8) << 3
            | ((c.coupling == Coupling::Dc) as u8) << 2
            | 2;
    }
    let first = [0x08, 0x00, b[0], b[1], b[2], b[3], 0x01, 0x00];
    let second = [
        0x08,
        0x00,
        b[0] & 0x86,
        b[1] & 0x86,
        b[2] & 0x86,
        b[3] & 0x86,
        0x01,
        0x00,
    ];
    (first, second)
}

// ---- trigger ----------------------------------------------------------------------------------

/// Trigger source / channel routing command.
pub fn cmd_trigger_source(timebase: TimeBase, mask: u8, source: usize, peak: bool) -> [u8; 6] {
    let t = timebase.index();
    let m = mask & 0xf;
    let n = m.count_ones();
    let (x, y): (u8, u8) = match n {
        1 => {
            let x = if t < 7 {
                0
            } else if t == 7 {
                1
            } else {
                (t != 8) as u8 + 2
            };
            (x, 3 - x)
        }
        2 => {
            let x = if t < 8 { 0 } else { (t != 8) as u8 + 1 };
            (x, 2 - x)
        }
        _ => {
            let x = (t > 8) as u8;
            (x, 1 - x)
        }
    };
    let uv6 = m >> source;
    let k = channel_mode(n as usize, timebase);
    let z: u8 = match k {
        1 => 0,
        2 => {
            if n == 1 {
                (!(m == 1 || m == 4)) as u8
            } else {
                (uv6 == 1) as u8
            }
        }
        _ => source as u8,
    };
    [
        0x12,
        0x00,
        (peak as u8) << 6 | (m << 2) | x,
        0x00,
        y & 3,
        ((!uv6) & 1) << 2 | z,
    ]
}

/// Trigger threshold command; `level` is an ADC code (0..=228), the hysteresis is +-4 codes.
pub fn cmd_trigger_level(level: u8) -> [u8; 26] {
    let mid = level as i32;
    let hi = (mid + 4).clamp(0, 0xe4) as u8;
    let lo = (mid - 4).max(0) as u8;
    let mut p = [0u8; 26];
    p[0] = 0x07;
    for g in 0..4 {
        p[2 + g * 4..6 + g * 4].copy_from_slice(&[hi, hi, lo, lo]);
    }
    p[18..].fill(mid as u8);
    p
}

/// Trigger mode command (`0x11`, DC trigger coupling): mode 0 = edge, 1 = pulse, 2 = video.
pub fn cmd_trigger_mode(mode: u8, slope: Slope) -> [u8; 6] {
    [0x11, 0x00, mode, (slope == Slope::Falling) as u8, 0x00, 0x00]
}

/// Edge trigger mode command (DC trigger coupling).
pub fn cmd_trigger_mode_edge(slope: Slope) -> [u8; 6] {
    cmd_trigger_mode(0, slope)
}

/// Trigger mode byte of a trigger kind.
pub fn trigger_mode_byte(kind: &TriggerKind) -> u8 {
    match kind {
        TriggerKind::Edge => 0,
        TriggerKind::Pulse { .. } => 1,
        TriggerKind::Video { .. } => 2,
    }
}

/// Pulse trigger command (`0x13`, vendor `dsoHTSetPulseTriger`), sent after the mode command.
///
/// The width is counted in units of 8 ns (a 125 MHz counter): `a = trunc(width_ns * 0.125)`. The vendor documentation
/// says the width argument is in units of 5 ns, which would give `a = width_ns / 40`; on a 6254BC that made the
/// hardware see pulses five times too wide (a 500 us pulse compared as greater than 2.4 ms but not 2.5 ms, for any
/// timebase), so the argument is in nanoseconds. The packet carries `a`, `1.05 a` and `0.95 a` (the tolerance window
/// used by "equal") as little-endian 32-bit values and the condition shifted left by one.
pub fn cmd_pulse_trigger(width_ns: u64, condition: PulseCondition) -> [u8; 16] {
    let a = (width_ns as f64 * 0.125).trunc();
    let upper = (a * 1.05).trunc() as u32;
    let lower = (a * 0.95).trunc() as u32;
    let cond: u8 = match condition {
        PulseCondition::Equal => 0,
        PulseCondition::NotEqual => 1,
        PulseCondition::Greater => 2,
        PulseCondition::Less => 3,
    };
    let mut p = [0u8; 16];
    p[0] = 0x13;
    p[2..6].copy_from_slice(&(a as u32).to_le_bytes());
    p[6..10].copy_from_slice(&upper.to_le_bytes());
    p[10..14].copy_from_slice(&lower.to_le_bytes());
    p[14] = cond << 1;
    p
}

/// Hardware line counter value for a 1-based video line number (vendor remapping, 16-bit arithmetic).
pub fn video_line_counter(standard: VideoStandard, line: u16) -> u16 {
    let l = line;
    match standard {
        VideoStandard::PalSecam => match l {
            0..=5 => l * 2 + 0x134,
            6..=0x136 => l + 0x13e,
            0x137..=0x13e => (l * 2).wrapping_sub(0x125),
            0x13f..=0x26e => l - 0x13f,
            0x26f..=0x271 => (l * 2).wrapping_sub(0x3ae),
            _ => l,
        },
        VideoStandard::Ntsc => match l {
            0..=9 => l * 2 + 0xfc,
            10..=0x107 => l + 0x106,
            0x108..=0x10f => l * 2 - 1,
            0x110..=0x20d => l - 0x110,
            _ => l,
        },
    }
}

/// Video trigger command (`0x16`, vendor `dsoHTSetVideoTriger`), sent after the mode command.
///
/// `level` is the trigger level as an ADC code, as for the edge trigger. The packet carries the line counter, the sync
/// selection, the level with +-2 hysteresis and fixed timing constants copied from the vendor code.
pub fn cmd_video_trigger(standard: VideoStandard, sync: VideoSync, positive: bool, level: u8) -> [u8; 16] {
    let (select, line) = match sync {
        VideoSync::AllLines => (0u8, 1u16),
        VideoSync::Line(n) => (1, n),
        VideoSync::OddField => (2, 1),
        VideoSync::EvenField => (3, 1),
        VideoSync::AllFields => (4, 1),
    };
    let counter = video_line_counter(standard, line);
    let mut p = [0u8; 16];
    p[0] = 0x16;
    p[2] = counter as u8;
    p[3] = (((positive as u8) << 3 | (select & 7)) << 3) | ((counter >> 8) as u8 & 3);
    p[4] = level.wrapping_add(2);
    p[5] = level.wrapping_sub(2);
    p[6..14].copy_from_slice(&[0x5b, 0x0e, 0x3b, 0x0d, 0x4b, 0x02, 0x2c, 0x01]);
    p[14] = 0; // logic trigger source, unused
    p[15] = 0x32;
    p
}

/// Horizontal trigger position / pre-trigger length command.
///
/// `interval_ns` is the sample interval, `buffer_len` the samples per channel, `percent` the trigger position
/// in the window and `fpga_version` selects the pipeline latency constant.
pub fn cmd_horizontal(interval_ns: f64, buffer_len: u32, percent: f64, fpga_version: u16) -> [u8; 14] {
    let latency = if matches!(fpga_version, 0xa000 | 0xa00a | 0xa00b) {
        246.0
    } else {
        230.0
    };
    let b = buffer_len as f64;
    let pre = (percent * b * 0.01).floor();
    let mut first = (latency + (pre + 100.0) * interval_ns) * 0.125;
    let mut second = (b - pre + 8.0) * interval_ns * 0.125;
    if interval_ns < 200.0 {
        first = first.ceil();
        second = second.ceil();
    } else {
        first = first.floor();
        second = second.floor();
    }
    let mut p = [0u8; 14];
    p[0] = 0x10;
    p[2..8].copy_from_slice(&(first as u64).to_le_bytes()[..6]);
    p[8..14].copy_from_slice(&(second as u64).to_le_bytes()[..6]);
    p
}

/// Pre-trigger samples for a window of `buffer_len` samples.
pub fn pre_trigger_samples(buffer_len: u32, percent: f64) -> u32 {
    (percent * buffer_len as f64 * 0.01).floor() as u32
}

// ---- amplitude (gain trim) calibration ----------------------------------------------------------

/// Per-unit gain trims stored in the device (`0xA2/0x1B00`), in 1/1024 units (`0x400` = 1.0).
///
/// Indexed by channel, row (the volts/div index remapped to 5, 7 or 9 for 0..=5, 6..=8 and 9..=11) and slot
/// (0 = one channel mode, 1 = two channel mode, 2 = four channel mode). The factory fills in the rows that matter;
/// the rest stay at `0x400`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmpCalibration {
    words: Vec<u16>,
}

impl Default for AmpCalibration {
    /// A table without any correction.
    fn default() -> Self {
        Self {
            words: vec![AMP_UNITY; AMP_WORDS],
        }
    }
}

impl AmpCalibration {
    /// Parse the bytes of an amplitude table read (at least `(AMP_WORDS + 1) * 2`). Returns `None` when the validity flag
    /// is wrong or the read is short; the vendor software then uses no correction either.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < (AMP_WORDS + 1) * 2 {
            return None;
        }
        let words: Vec<u16> = bytes
            .chunks_exact(2)
            .take(AMP_WORDS + 1)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if words[AMP_WORDS] != AMP_FLAG {
            return None;
        }
        Some(Self {
            words: words[..AMP_WORDS].to_vec(),
        })
    }

    /// The raw words (without the flag).
    pub fn words(&self) -> &[u16] {
        &self.words
    }

    /// Trim of channel `ch` (0..=3) for volts/div index `v` and channel mode `k` (1, 2 or 4).
    pub fn trim(&self, ch: usize, v: u8, k: u8) -> u16 {
        let row = match v {
            0..=5 => 5,
            6..=8 => 7,
            _ => 9,
        };
        let slot = match k {
            1 => 0,
            2 => 1,
            _ => 2,
        };
        self.words[(ch * 12 + row) * 6 + slot]
    }
}

/// The 26-byte amplitude calibration command (`0x1E`, vendor `dsoHTSetAmpCalibrate`).
///
/// `mask` is the enabled-channel bitmask, `k` the channel mode, `vdiv` the volts/div index and `position` the 0..=255
/// vertical position of each channel. The header carries four trims; for each trim other than unity the packet also
/// carries an offset correction `trunc(p' * (1024 - t) + 0.5)` with `p' = trunc((position - 128) * 0.78125 + 128.5)`
/// that keeps the zero level in place when the gain changes. Which channel supplies the trim and position of each
/// slot follows the vendor code: one channel mode uses the enabled channel for all four, two channel mode uses the
/// first enabled channel's trim for all four and its position for the first pair, the second's for the second pair.
pub fn cmd_amp_calibration(cal: &AmpCalibration, mask: u8, k: u8, vdiv: [u8; 4], position: [u8; 4]) -> [u8; 26] {
    let enabled: Vec<usize> = (0..4).filter(|i| mask >> i & 1 != 0).collect();
    let (trims, pos): ([u16; 4], [u8; 4]) = match k {
        1 => {
            let e = enabled.first().copied().unwrap_or(0);
            ([cal.trim(e, vdiv[e], 1); 4], [position[e]; 4])
        }
        2 => {
            let f = enabled.first().copied().unwrap_or(0);
            let s = enabled.get(1).copied().unwrap_or(f);
            (
                [cal.trim(f, vdiv[f], 2); 4],
                [position[f], position[f], position[s], position[s]],
            )
        }
        _ => ([0, 1, 2, 3].map(|i| cal.trim(i, vdiv[i], 4)), position),
    };
    let mut out = [0u8; 26];
    out[0] = 0x1e;
    for i in 0..4 {
        out[2 + 2 * i..4 + 2 * i].copy_from_slice(&trims[i].to_le_bytes());
        if trims[i] != AMP_UNITY {
            let scaled = ((f64::from(pos[i]) - 128.0) * 0.78125 + 128.5).trunc();
            let mut g = (scaled * (1024.0 - f64::from(trims[i])) + 0.5).trunc() as i64;
            if g < 0 {
                g += 0x200_0000;
            }
            out[10 + 4 * i..14 + 4 * i].copy_from_slice(&(g as u32).to_le_bytes());
        }
    }
    out
}

// ---- calibration and position -----------------------------------------------------------------

/// The calibration table stored in the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calibration {
    words: Vec<u16>,
}

impl Calibration {
    /// Parse the 1154 bytes returned by the calibration read and check the validity flag.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < CAL_WORDS * 2 {
            return Err(Error::Protocol(format!(
                "calibration read returned {} bytes",
                bytes.len()
            )));
        }
        let words: Vec<u16> = bytes
            .chunks_exact(2)
            .take(CAL_WORDS)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if words[CAL_WORDS - 1] != CAL_FLAG {
            return Err(Error::BadCalibration);
        }
        Ok(Self { words })
    }

    /// Build a table from words (tests, offline use); the flag word is forced to the valid value.
    pub fn from_words(mut words: Vec<u16>) -> Self {
        words.resize(CAL_WORDS, 0);
        words[CAL_WORDS - 1] = CAL_FLAG;
        Self { words }
    }

    /// The raw words.
    pub fn words(&self) -> &[u16] {
        &self.words
    }

    /// Vertical position word for channel `ch` (0..=3), volts/div index `v`, channel mode `k` and position 0..=255.
    pub fn position_word(&self, ch: usize, v: u8, k: u8, position: u8) -> u16 {
        let row = if v <= 5 {
            5
        } else if v <= 8 {
            8
        } else {
            11
        };
        let m = match k {
            4 => 4,
            2 => 2,
            _ => 0,
        };
        let idx = m + 12 * (12 * ch + row);
        let (a, b) = (self.words[idx] as i64, self.words[idx + 1] as i64);
        let (lo, hi) = if matches!(v, 5 | 8 | 11) {
            (a, b)
        } else {
            let avg = (a + b + 1) >> 1;
            let mul = match v {
                0 => 0.02,
                1 => 0.05,
                2 => 0.1,
                _ => 0.2,
            };
            let d = (((b - avg) as f64) * mul + 0.5).floor() as i64;
            (avg - d, avg + d)
        };
        (((hi - lo) as f64 * position as f64 / 255.0 + lo as f64) as i64) as u16
    }
}

/// Channel position command: id byte is `0` for CH1, then `1 << (ch - 1)`.
pub fn cmd_channel_position(ch: usize, word: u16) -> [u8; 4] {
    let id = if ch == 0 { 0 } else { 1u8 << (ch - 1) };
    let w = word.to_le_bytes();
    [id, 0x00, w[0], w[1]]
}

/// Split the 71-byte info string into `(name, pcb_version, serial)` using the vendor software's slices.
pub fn parse_info(raw: &[u8]) -> (String, String, String) {
    let field = |a: usize, b: usize| -> String {
        raw.get(a..b.min(raw.len()))
            .unwrap_or(&[])
            .iter()
            .take_while(|&&c| c != 0xff && c != 0)
            .map(|&c| c as char)
            .collect()
    };
    (field(6, 14), field(14, 20), field(34, 42))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn tb(i: u8) -> TimeBase {
        TimeBase::from_index(i).unwrap()
    }

    #[test]
    fn timebase_golden() {
        assert_eq!(cmd_timebase(tb(15)), [0x0f, 0, 99, 0, 0, 0]);
        assert_eq!(cmd_timebase(tb(0)), [0x0f, 0, 0, 0, 0, 0]);
        assert_eq!(cmd_timebase(tb(35)), [0x0f, 0, 0xff, 0x64, 0xcd, 0x1d]); // 500_000_000 - 1
    }

    #[test]
    fn horizontal_matches_vendor_and_open_source_constants() {
        let p = cmd_horizontal(800.0, 4096, 50.0, 0xd001);
        assert_eq!(&p[..2], &[0x10, 0x00]);
        assert_eq!(u64::from_le_bytes([p[2], p[3], p[4], p[5], p[6], p[7], 0, 0]), 214828);
        assert_eq!(
            u64::from_le_bytes([p[8], p[9], p[10], p[11], p[12], p[13], 0, 0]),
            205600
        );
        let p10 = cmd_horizontal(800.0, 4096, 10.0, 0xd001);
        assert_eq!(
            u64::from_le_bytes([p10[2], p10[3], p10[4], p10[5], p10[6], p10[7], 0, 0]),
            50928
        );
        assert_eq!(
            u64::from_le_bytes([p10[8], p10[9], p10[10], p10[11], p10[12], p10[13], 0, 0]),
            369500
        );
    }

    #[test]
    fn relay_bytes_1v_dc() {
        let ch = [ChannelConfig::default(); 4];
        let (a, b) = cmd_relays(&ch);
        assert_eq!(a, [8, 0, 0x36, 0x36, 0x36, 0x36, 1, 0]);
        assert_eq!(b, [8, 0, 0x06, 0x06, 0x06, 0x06, 1, 0]);
    }

    #[test]
    fn relay_bytes_bw_limit_and_ac() {
        let mut ch = [ChannelConfig::default(); 4];
        ch[0].bandwidth_limit = true;
        ch[1].coupling = Coupling::Ac;
        ch[2].volts_per_div = VoltsPerDiv::from_index(0).unwrap();
        ch[3].volts_per_div = VoltsPerDiv::from_index(11).unwrap();
        let (a, _) = cmd_relays(&ch);
        assert_eq!(a[2], 0xb6); // bw + <=1V + >100mV + DC + bit1
        assert_eq!(a[3], 0x32); // AC
        assert_eq!(a[4], 0x2e); // 2 mV: <=1V, <=100mV, DC
        assert_eq!(a[5], 0x56); // 10 V: >1V, >100mV, DC
    }

    #[test]
    fn trigger_level_packet() {
        let p = cmd_trigger_level(128);
        assert_eq!(&p[..6], &[7, 0, 132, 132, 124, 124]);
        assert_eq!(&p[18..], &[128u8; 8]);
        assert_eq!(cmd_trigger_level(228)[2], 0xe4);
    }

    #[test]
    fn trigger_source_4ch() {
        assert_eq!(cmd_trigger_source(tb(15), 0xf, 0, false), [0x12, 0, 0x3d, 0, 0, 0]);
    }

    #[test]
    fn adc_quad_and_single() {
        let quad = adc_register_writes(0xf, [8; 4], tb(15));
        assert_eq!(
            quad,
            vec![(0x3a, 0x0810), (0x3b, 0x0204), (0x0f, 0), (0x31, 0x0204), (0x2a, 0)]
        );
        let single = adc_register_writes(0x4, [8; 4], tb(3));
        assert_eq!(
            single,
            vec![(0x3a, 0x0404), (0x3b, 0x0404), (0x0f, 0), (0x31, 0x0001), (0x2b, 0)]
        );
        // 10 mV on CH1 in quad mode: gain code 7 in the lowest nibble
        let g = adc_register_writes(0xf, [2, 8, 8, 8], tb(15));
        assert_eq!(g[4], (0x2a, 0x0007));
    }

    #[test]
    fn channel_modes() {
        assert_eq!(channel_mode(1, tb(3)), 1);
        assert_eq!(channel_mode(1, tb(7)), 2);
        assert_eq!(channel_mode(1, tb(8)), 4);
        assert_eq!(channel_mode(2, tb(7)), 2);
        assert_eq!(channel_mode(2, tb(8)), 4);
        assert_eq!(channel_mode(3, tb(0)), 4);
    }

    #[test]
    fn sample_interval() {
        assert_eq!(sample_interval_ns(tb(15), 4), 800.0);
        assert_eq!(sample_interval_ns(tb(8), 4), 4.0);
        assert_eq!(sample_interval_ns(tb(3), 1), 1.0);
    }

    #[test]
    fn position_word_from_calibration() {
        // CH1, 1 V/div (row 8), quad mode (offset 4): index 4 + 12*8 = 100
        let mut w = vec![0u16; CAL_WORDS];
        w[100] = 13832;
        w[101] = 44132;
        let cal = Calibration::from_words(w);
        assert_eq!(cal.position_word(0, 8, 4, 128), 29041);
        assert_eq!(cal.position_word(0, 8, 4, 0), 13832);
        assert_eq!(cal.position_word(0, 8, 4, 255), 44132);
        assert_eq!(cmd_channel_position(0, 29041), [0, 0, 0x71, 0x71]);
        assert_eq!(cmd_channel_position(3, 0x72d9), [4, 0, 0xd9, 0x72]);
    }

    #[test]
    fn calibration_flag_checked() {
        let mut b = vec![0u8; CAL_WORDS * 2];
        assert!(matches!(Calibration::from_bytes(&b), Err(Error::BadCalibration)));
        b[CAL_WORDS * 2 - 2] = 0xcf;
        b[CAL_WORDS * 2 - 1] = 0xfb;
        assert!(Calibration::from_bytes(&b).is_ok());
    }

    #[test]
    fn spi_and_init() {
        assert_eq!(spi_write(SpiChip::Adc, 0x12, 0x4777), [8, 0, 0, 0x77, 0x47, 0x12, 4, 0]);
        assert_eq!(spi_write(SpiChip::Pll, 0x0f, 0xf128), [8, 0, 0, 0x28, 0xf1, 0x0f, 2, 0]);
    }

    #[test]
    fn pcb_and_info() {
        assert_eq!(parse_pcb_revision(b"P1.06\xff"), 106);
        assert_eq!(adc_gain_register(106, 4), 0x3f);
        let mut raw = vec![0xffu8; 71];
        raw[..3].copy_from_slice(b"DSO");
        raw[6..10].copy_from_slice(b"6000");
        raw[14..19].copy_from_slice(b"P1.06");
        let (name, pcb, _) = parse_info(&raw);
        assert_eq!((name.as_str(), pcb.as_str()), ("6000", "P1.06"));
    }

    #[test]
    fn small_commands() {
        assert_eq!(cmd_start(Sweep::Auto), [3, 0, 1, 0]);
        assert_eq!(cmd_start(Sweep::Normal), [3, 0, 0, 0]);
        assert_eq!(cmd_readback(16384), [5, 0, 0x00, 0x20]);
        assert_eq!(cmd_readback(65536), [5, 0, 0x00, 0x80]);
        assert_eq!(cmd_set_read_address(0x1234), [0x0e, 0, 0x34, 0x12]);
        assert_eq!(cmd_trigger_mode_edge(Slope::Falling), [0x11, 0, 0, 1, 0, 0]);
        assert!(State::parse(&[0x03, 0xc4]).unwrap().triggered());
        assert!(State::parse(&[0x02, 0xc4]).unwrap().data_ready());
        assert_eq!(frame_stride(3), 4);
        assert_eq!(frame_stride(2), 2);
    }

    #[test]
    fn amp_calibration_packet_matches_hardware_experiment() {
        // CH2 only, 500 mV/div (row 7), trim 0x440, position 128: verified on a 6254BC (gain x1.0625, zero unchanged)
        let mut words = vec![AMP_UNITY; AMP_WORDS];
        words[(12 + 7) * 6] = 0x440;
        let cal = AmpCalibration { words };
        let pkt = cmd_amp_calibration(&cal, 0b0010, 1, [7; 4], [128; 4]);
        let expected: [u8; 26] = [
            0x1e, 0, 0x40, 4, 0x40, 4, 0x40, 4, 0x40, 4, 0x01, 0xe0, 0xff, 0x01, 0x01, 0xe0, 0xff, 0x01, 0x01, 0xe0,
            0xff, 0x01, 0x01, 0xe0, 0xff, 0x01,
        ];
        assert_eq!(pkt, expected);
        // unity everywhere: header only, no offset words
        let none = cmd_amp_calibration(&AmpCalibration::default(), 0b0010, 1, [7; 4], [128; 4]);
        assert!(none[10..].iter().all(|&b| b == 0));
    }

    #[test]
    fn amp_table_validity_and_rows() {
        let mut b = vec![0u8; (AMP_WORDS + 1) * 2];
        assert!(AmpCalibration::from_bytes(&b).is_none());
        b[AMP_WORDS * 2..].copy_from_slice(&AMP_FLAG.to_le_bytes());
        let cal = AmpCalibration::from_bytes(&b).unwrap();
        assert_eq!(cal.trim(0, 8, 4), 0);
        let d = AmpCalibration::default();
        assert_eq!(d.trim(3, 11, 2), AMP_UNITY);
    }

    #[test]
    fn pulse_trigger_packet() {
        // 500 us, "equal": a = 62500 units of 8 ns (0xf424), 1.05a = 65625 (0x10059), 0.95a = 59375 (0xe7ef)
        let p = cmd_pulse_trigger(500_000, PulseCondition::Equal);
        assert_eq!(p, [0x13, 0, 0x24, 0xf4, 0, 0, 0x59, 0, 1, 0, 0xef, 0xe7, 0, 0, 0, 0]);
        assert_eq!(cmd_pulse_trigger(500_000, PulseCondition::Less)[14], 6);
        assert_eq!(cmd_pulse_trigger(500_000, PulseCondition::Greater)[14], 4);
        assert_eq!(cmd_pulse_trigger(500_000, PulseCondition::NotEqual)[14], 2);
    }

    #[test]
    fn video_trigger_packet_and_line_counter() {
        // PAL, all lines (line 1 -> counter 310 = 0x136), positive sync, level 128
        let p = cmd_video_trigger(VideoStandard::PalSecam, VideoSync::AllLines, true, 128);
        assert_eq!(
            p,
            [0x16, 0, 0x36, 0x41, 0x82, 0x7e, 0x5b, 0x0e, 0x3b, 0x0d, 0x4b, 0x02, 0x2c, 0x01, 0, 0x32]
        );
        assert_eq!(video_line_counter(VideoStandard::PalSecam, 1), 310);
        assert_eq!(video_line_counter(VideoStandard::PalSecam, 6), 0x144);
        assert_eq!(video_line_counter(VideoStandard::PalSecam, 0x140), 1);
        assert_eq!(video_line_counter(VideoStandard::Ntsc, 1), 254);
        assert_eq!(video_line_counter(VideoStandard::Ntsc, 0x110), 0);
        assert_eq!(trigger_mode_byte(&TriggerKind::Edge), 0);
        assert_eq!(cmd_trigger_mode(2, Slope::Falling), [0x11, 0, 2, 1, 0, 0]);
    }

    #[test]
    fn driver_version_is_bcd() {
        assert_eq!(
            parse_driver_version(&[0x20, 0x16, 0x05, 0x31]).as_deref(),
            Some("20160531")
        );
        assert_eq!(parse_driver_version(&[0xff, 0xff, 0xff, 0xff]), None);
        assert_eq!(parse_driver_version(&[0x20]), None);
    }
}
