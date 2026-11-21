//! `hantek6xx4`: command line tool for Hantek 6xx4-series USB oscilloscopes.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use hantek6xx4_lib::{
    CaptureOptions, Channel, ChannelConfig, Config, Coupling, PulseCondition, Scope, Slope, Sweep, TimeBase, Transport,
    TriggerKind, UsbTransport, VideoStandard, VideoSync, VoltsPerDiv,
};
use std::io::Write;
use std::time::Duration;

#[derive(Parser)]
#[command(version, about = "Talk to Hantek 6xx4-series USB oscilloscopes")]
struct Cli {
    /// Which connected scope to use (0 based)
    #[arg(long, global = true, default_value_t = 0)]
    device: usize,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List connected scopes
    List,
    /// Initialise the scope and print identification data
    Info,
    /// Read and print the acquisition state word
    State,
    /// Save the calibration table (read only) to a file
    DumpCalibration {
        /// Output file (raw little-endian 16-bit words)
        file: String,
    },
    /// Read the factory amplitude (gain trim) table and save it (read-only backup)
    DumpAmpCalibration {
        /// Output file (raw little-endian 16-bit words, followed by the 0xFACF validity word)
        file: String,
    },
    /// Configure the scope and capture one frame
    Capture(CaptureArgs),
    /// Send raw commands (low level, for protocol exploration)
    Raw {
        #[command(subcommand)]
        what: RawCmd,
    },
    /// Print the supported volts/div and time base settings
    Ranges,
}

#[derive(Subcommand)]
enum RawCmd {
    /// Bulk command (with the control preamble), optionally read a reply
    Bulk {
        /// Hex bytes, e.g. "06 00"
        hex: String,
        /// Read this many reply bytes
        #[arg(long)]
        read: Option<usize>,
    },
    /// Vendor control read
    ControlIn { request: String, value: String, len: usize },
    /// Vendor control write (use with care)
    ControlOut {
        request: String,
        value: String,
        hex: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Statistics per channel
    Summary,
    /// Comma separated values, one row per sample
    Csv,
    /// Raw sample bytes, channels one after the other
    Raw,
}

#[derive(Clone, Copy, ValueEnum)]
enum TriggerType {
    Edge,
    Pulse,
    /// Implemented from the vendor code; not verified against a video signal
    Video,
}

#[derive(Clone, Copy, ValueEnum)]
enum PulseCond {
    /// Equal (+-5 %)
    Eq,
    Ne,
    /// Wider than
    Gt,
    /// Narrower than
    Lt,
}

#[derive(Clone, Copy, ValueEnum)]
enum VideoStd {
    Pal,
    Ntsc,
}

#[derive(Clone, Copy, ValueEnum)]
enum VideoSyncArg {
    AllLines,
    Line,
    Odd,
    Even,
    AllFields,
}

/// Parse a duration such as "500us", "20ns", "1.5ms" or "2s" into nanoseconds.
fn parse_duration_ns(s: &str) -> Result<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(n) = s.strip_suffix("ns") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix("us") {
        (n, 1e3)
    } else if let Some(n) = s.strip_suffix("ms") {
        (n, 1e6)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1e9)
    } else {
        bail!("duration '{s}' needs a unit: ns, us, ms or s");
    };
    let v: f64 = num.trim().parse().map_err(|e| anyhow!("bad duration '{s}': {e}"))?;
    if !(v.is_finite() && v >= 0.0) {
        bail!("bad duration '{s}'");
    }
    Ok((v * mult).round() as u64)
}

#[derive(Args)]
struct CaptureArgs {
    /// Channels to enable, e.g. "1" or "1,3,4"
    #[arg(long, default_value = "1")]
    channels: String,
    /// Volts per division for all channels (label like 200mV or index 0..11)
    #[arg(long, default_value = "1V")]
    vdiv: String,
    /// Time base (label like 200us or index 0..35)
    #[arg(long, default_value = "200us")]
    timebase: String,
    /// Vertical position 0..255
    #[arg(long, default_value_t = 128)]
    position: u8,
    /// Use AC coupling
    #[arg(long)]
    ac: bool,
    /// Enable the bandwidth limit
    #[arg(long)]
    bw_limit: bool,
    /// Trigger source channel 1..4 (default: first enabled channel)
    #[arg(long)]
    trigger_source: Option<usize>,
    /// Trigger level as ADC code 0..228
    #[arg(long, default_value_t = 128)]
    trigger_level: u8,
    /// Use the falling edge (for the pulse trigger: negative pulses)
    #[arg(long)]
    falling: bool,
    /// Trigger type
    #[arg(long, value_enum, default_value_t = TriggerType::Edge)]
    trigger_type: TriggerType,
    /// Pulse trigger width, e.g. 500us, 20ns, 1.5ms, 2s (10ns..10s)
    #[arg(long)]
    pulse_width: Option<String>,
    /// Pulse trigger condition
    #[arg(long, value_enum, default_value_t = PulseCond::Eq)]
    pulse_cond: PulseCond,
    /// Video standard
    #[arg(long, value_enum, default_value_t = VideoStd::Pal)]
    video_standard: VideoStd,
    /// Video sync
    #[arg(long, value_enum, default_value_t = VideoSyncArg::AllLines)]
    video_sync: VideoSyncArg,
    /// Video line number for --video-sync line (1-based)
    #[arg(long, default_value_t = 1)]
    video_line: u16,
    /// Negative video sync polarity (default positive)
    #[arg(long)]
    video_negative: bool,
    /// Wait for a trigger instead of free running
    #[arg(long)]
    normal: bool,
    /// Trigger position in the window, percent
    #[arg(long, default_value_t = 50.0)]
    hpos: f64,
    /// Capture timeout in milliseconds
    #[arg(long, default_value_t = 2000)]
    timeout_ms: u64,
    #[arg(long, value_enum, default_value_t = Format::Summary)]
    format: Format,
    /// Print volts instead of raw ADC codes (summary and csv formats)
    #[arg(long)]
    volts: bool,
    /// Write output to this file instead of stdout
    #[arg(short, long)]
    output: Option<String>,
}

fn parse_num<T: TryFrom<u64>>(s: &str) -> Result<T> {
    let v = if let Some(h) = s.strip_prefix("0x") {
        u64::from_str_radix(h, 16)
    } else {
        s.parse::<u64>()
    }
    .with_context(|| format!("bad number '{s}'"))?;
    T::try_from(v).map_err(|_| anyhow!("number '{s}' out of range"))
}

fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    let clean = clean.trim_start_matches("0x");
    if clean.len() % 2 != 0 {
        bail!("hex string needs an even number of digits");
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).map_err(Into::into))
        .collect()
}

fn hexdump(b: &[u8]) -> String {
    b.chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let h: Vec<String> = c.iter().map(|x| format!("{x:02x}")).collect();
            let a: String = c
                .iter()
                .map(|&x| if (32..127).contains(&x) { x as char } else { '.' })
                .collect();
            format!("{:04x}  {:<48} {}", i * 16, h.join(" "), a)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn open(device: usize) -> Result<Scope<UsbTransport>> {
    Scope::open_nth(device).context("opening the scope")
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::List => {
            let devs = UsbTransport::enumerate()?;
            if devs.is_empty() {
                println!("no Hantek 6xx4 devices found");
            }
            for (i, (bus, addr)) in devs.iter().enumerate() {
                println!("{i}: Hantek 6xx4 series (04b5:6cde) on bus {bus} address {addr}");
            }
        }
        Cmd::Info => {
            let s = open(cli.device)?;
            let i = s.info();
            println!("family:       {}", i.name);
            println!("PCB revision: {}", i.pcb_version);
            println!("serial field: {}", i.serial);
            println!("FPGA version: 0x{:04x}", i.fpga_version);
            println!("raw info:\n{}", hexdump(&i.raw_info));
            println!("calibration:  valid ({} words)", s.calibration().words().len());
        }
        Cmd::State => {
            let mut s = open(cli.device)?;
            let st = s.state()?;
            println!(
                "state 0x{:04x}  triggered={} data_ready={}",
                st.0,
                st.triggered(),
                st.data_ready()
            );
        }
        Cmd::DumpCalibration { file } => {
            let s = open(cli.device)?;
            let bytes: Vec<u8> = s.calibration().words().iter().flat_map(|w| w.to_le_bytes()).collect();
            std::fs::write(&file, bytes)?;
            println!("wrote {} words to {file}", s.calibration().words().len());
        }
        Cmd::DumpAmpCalibration { file } => {
            let s = open(cli.device)?;
            let words = s.amp_calibration().words();
            let bytes: Vec<u8> = words
                .iter()
                .copied()
                .chain(std::iter::once(hantek6xx4_lib::protocol::AMP_FLAG))
                .flat_map(|w| w.to_le_bytes())
                .collect();
            std::fs::write(&file, bytes)?;
            let trimmed = words
                .iter()
                .filter(|&&w| w != hantek6xx4_lib::protocol::AMP_UNITY)
                .count();
            println!(
                "wrote {} words to {file} ({trimmed} differ from unity)",
                words.len() + 1
            );
        }
        Cmd::Ranges => {
            println!(
                "volts/div: {}",
                (0..12)
                    .map(|i| VoltsPerDiv::from_index(i).unwrap().label())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            println!("time base:");
            for i in 0..TimeBase::COUNT {
                println!("  {i:2}: {}", TimeBase::from_index(i)?.label());
            }
        }
        Cmd::Raw { what } => {
            let mut s = open(cli.device)?;
            match what {
                RawCmd::Bulk { hex, read } => {
                    s.send(&parse_hex(&hex)?)?;
                    if let Some(n) = read {
                        println!("{}", hexdump(&s.recv(n)?));
                    }
                }
                RawCmd::ControlIn { request, value, len } => {
                    let r = s
                        .transport_mut()
                        .control_in(parse_num(&request)?, parse_num(&value)?, len)?;
                    println!("{}", hexdump(&r));
                }
                RawCmd::ControlOut { request, value, hex } => {
                    s.transport_mut()
                        .control_out(parse_num(&request)?, parse_num(&value)?, &parse_hex(&hex)?)?;
                }
            }
        }
        Cmd::Capture(a) => capture(cli.device, a)?,
    }
    Ok(())
}

fn capture(device: usize, a: CaptureArgs) -> Result<()> {
    let mut cfg = Config::default();
    let vdiv: VoltsPerDiv = a.vdiv.parse()?;
    let enabled: Vec<usize> = a
        .channels
        .split(',')
        .map(|c| c.trim().parse::<usize>().map_err(|e| anyhow!("bad channel '{c}': {e}")))
        .collect::<Result<_>>()?;
    for (i, c) in cfg.channels.iter_mut().enumerate() {
        *c = ChannelConfig {
            enabled: enabled.contains(&(i + 1)),
            volts_per_div: vdiv,
            coupling: if a.ac { Coupling::Ac } else { Coupling::Dc },
            bandwidth_limit: a.bw_limit,
            position: a.position,
        };
    }
    cfg.timebase = a.timebase.parse()?;
    let source = a
        .trigger_source
        .or_else(|| enabled.iter().copied().min())
        .ok_or_else(|| anyhow!("no channel enabled"))?;
    cfg.trigger.source = Channel::from_index(source.checked_sub(1).ok_or_else(|| anyhow!("trigger source is 1..4"))?)?;
    cfg.trigger.level = a.trigger_level;
    cfg.trigger.slope = if a.falling { Slope::Falling } else { Slope::Rising };
    cfg.trigger.sweep = if a.normal { Sweep::Normal } else { Sweep::Auto };
    cfg.trigger.horizontal_position_percent = a.hpos;
    cfg.trigger.kind = match a.trigger_type {
        TriggerType::Edge => TriggerKind::Edge,
        TriggerType::Pulse => TriggerKind::Pulse {
            width_ns: parse_duration_ns(
                a.pulse_width
                    .as_deref()
                    .ok_or_else(|| anyhow!("--pulse-width is required"))?,
            )?,
            condition: match a.pulse_cond {
                PulseCond::Eq => PulseCondition::Equal,
                PulseCond::Ne => PulseCondition::NotEqual,
                PulseCond::Gt => PulseCondition::Greater,
                PulseCond::Lt => PulseCondition::Less,
            },
        },
        TriggerType::Video => TriggerKind::Video {
            standard: match a.video_standard {
                VideoStd::Pal => VideoStandard::PalSecam,
                VideoStd::Ntsc => VideoStandard::Ntsc,
            },
            sync: match a.video_sync {
                VideoSyncArg::AllLines => VideoSync::AllLines,
                VideoSyncArg::Line => VideoSync::Line(a.video_line),
                VideoSyncArg::Odd => VideoSync::OddField,
                VideoSyncArg::Even => VideoSync::EvenField,
                VideoSyncArg::AllFields => VideoSync::AllFields,
            },
            positive: !a.video_negative,
        },
    };

    let mut scope = open(device)?;
    scope.configure(&cfg)?;
    let frame = scope.capture(CaptureOptions {
        timeout: Duration::from_millis(a.timeout_ms),
    })?;

    let mut out: Box<dyn Write> = match &a.output {
        Some(f) => Box::new(std::fs::File::create(f)?),
        None => Box::new(std::io::stdout().lock()),
    };
    match a.format {
        Format::Summary => {
            writeln!(
                out,
                "triggered={} sample interval={} ns trigger sample={:?}",
                frame.triggered, frame.sample_interval_ns, frame.trigger_sample
            )?;
            for ch in Channel::ALL {
                if a.volts {
                    if let Some(v) = frame.volts(ch) {
                        let n = v.len() as f64;
                        let mean = v.iter().sum::<f64>() / n;
                        let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
                        let min = v.iter().cloned().fold(f64::INFINITY, f64::min);
                        let max = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                        writeln!(
                            out,
                            "CH{}: {} samples  min {:.3} V  max {:.3} V  pp {:.3} V  mean {:.3} V  sd {:.3} V",
                            ch.index() + 1,
                            v.len(),
                            min,
                            max,
                            max - min,
                            mean,
                            sd
                        )?;
                    }
                } else if let Some(s) = frame.channel(ch) {
                    let n = s.len() as f64;
                    let mean = s.iter().map(|&x| x as f64).sum::<f64>() / n;
                    let sd = (s.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n).sqrt();
                    writeln!(
                        out,
                        "CH{}: {} samples  min {}  max {}  mean {:.1}  sd {:.2}",
                        ch.index() + 1,
                        s.len(),
                        s.iter().min().unwrap(),
                        s.iter().max().unwrap(),
                        mean,
                        sd
                    )?;
                }
            }
        }
        Format::Csv => {
            let chans: Vec<Channel> = Channel::ALL
                .into_iter()
                .filter(|&c| frame.channel(c).is_some())
                .collect();
            let header: Vec<String> = std::iter::once("time_ns".to_string())
                .chain(
                    chans
                        .iter()
                        .map(|c| format!("ch{}{}", c.index() + 1, if a.volts { "_v" } else { "" })),
                )
                .collect();
            writeln!(out, "{}", header.join(","))?;
            let len = frame.channel(chans[0]).map(|s| s.len()).unwrap_or(0);
            for i in 0..len {
                let row: Vec<String> = std::iter::once(format!("{}", i as f64 * frame.sample_interval_ns))
                    .chain(chans.iter().map(|&c| {
                        let code = frame.channel(c).unwrap()[i];
                        if a.volts {
                            format!("{:.4}", frame.code_to_volts(c, code))
                        } else {
                            code.to_string()
                        }
                    }))
                    .collect();
                writeln!(out, "{}", row.join(","))?;
            }
        }
        Format::Raw => {
            for ch in Channel::ALL {
                if let Some(s) = frame.channel(ch) {
                    out.write_all(s)?;
                }
            }
        }
    }
    Ok(())
}
