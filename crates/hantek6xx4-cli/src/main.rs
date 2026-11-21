//! `hantek6xx4`: command line tool for Hantek 6xx4-series USB oscilloscopes.

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use hantek6xx4_lib::{
    CaptureOptions, Channel, ChannelConfig, Config, Coupling, Scope, Slope, Sweep, TimeBase, Transport, UsbTransport,
    VoltsPerDiv,
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
    /// Use the falling edge
    #[arg(long)]
    falling: bool,
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
                if let Some(s) = frame.channel(ch) {
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
                .chain(chans.iter().map(|c| format!("ch{}", c.index() + 1)))
                .collect();
            writeln!(out, "{}", header.join(","))?;
            let len = frame.channel(chans[0]).map(|s| s.len()).unwrap_or(0);
            for i in 0..len {
                let row: Vec<String> = std::iter::once(format!("{}", i as f64 * frame.sample_interval_ns))
                    .chain(chans.iter().map(|&c| frame.channel(c).unwrap()[i].to_string()))
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
