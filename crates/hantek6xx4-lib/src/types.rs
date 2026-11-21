//! User-facing configuration and result types.

use crate::error::{Error, Result};
use std::str::FromStr;

/// Number of analog channels.
pub const MAX_CHANNELS: usize = 4;

/// An analog input channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Ch1,
    Ch2,
    Ch3,
    Ch4,
}

impl Channel {
    /// All channels in order.
    pub const ALL: [Channel; MAX_CHANNELS] = [Channel::Ch1, Channel::Ch2, Channel::Ch3, Channel::Ch4];

    /// Zero based index (CH1 = 0).
    pub fn index(self) -> usize {
        self as usize
    }

    /// Channel from a zero based index.
    pub fn from_index(i: usize) -> Result<Self> {
        Self::ALL
            .get(i)
            .copied()
            .ok_or_else(|| Error::invalid(format!("channel index {i} (0..=3)")))
    }
}

/// Vertical sensitivity, 1-2-5 sequence from 2 mV to 10 V per division.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoltsPerDiv(u8);

const VDIV_LABELS: [&str; 12] = [
    "2mV", "5mV", "10mV", "20mV", "50mV", "100mV", "200mV", "500mV", "1V", "2V", "5V", "10V",
];
const VDIV_VOLTS: [f64; 12] = [0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0];

impl VoltsPerDiv {
    pub const COUNT: u8 = 12;
    pub const V1: VoltsPerDiv = VoltsPerDiv(8);

    /// From the device index (0 = 2 mV ... 11 = 10 V).
    pub fn from_index(i: u8) -> Result<Self> {
        if i < Self::COUNT {
            Ok(Self(i))
        } else {
            Err(Error::invalid(format!("volts/div index {i} (0..=11)")))
        }
    }
    /// Device index (0 = 2 mV ... 11 = 10 V).
    pub fn index(self) -> u8 {
        self.0
    }
    /// Volts per division.
    pub fn volts(self) -> f64 {
        VDIV_VOLTS[self.0 as usize]
    }
    /// Human readable label such as `"200mV"`.
    pub fn label(self) -> &'static str {
        VDIV_LABELS[self.0 as usize]
    }
}

impl FromStr for VoltsPerDiv {
    type Err = Error;
    /// Accepts a label (`"200mV"`, case-insensitive) or a device index (`"7"`).
    fn from_str(s: &str) -> Result<Self> {
        if let Some(i) = VDIV_LABELS.iter().position(|l| l.eq_ignore_ascii_case(s)) {
            return Ok(Self(i as u8));
        }
        s.parse::<u8>()
            .map_err(|_| Error::invalid(format!("unknown volts/div '{s}'")))
            .and_then(Self::from_index)
    }
}

/// Horizontal time base, 2 ns to 1000 s per division in a 1-2-5 sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBase(u8);

impl TimeBase {
    pub const COUNT: u8 = 36;

    /// From the device index (0 = 2 ns/div ... 35 = 1000 s/div).
    pub fn from_index(i: u8) -> Result<Self> {
        if i < Self::COUNT {
            Ok(Self(i))
        } else {
            Err(Error::invalid(format!("time base index {i} (0..=35)")))
        }
    }
    /// Device index.
    pub fn index(self) -> u8 {
        self.0
    }
    /// Nanoseconds per division (exact, computed in integers).
    pub fn ns_per_div(self) -> f64 {
        let (k, r) = ((self.0 / 3) as u32, (self.0 % 3) as usize);
        ([2u64, 5, 10][r] * 10u64.pow(k)) as f64
    }
    /// Seconds per division.
    pub fn seconds_per_div(self) -> f64 {
        self.ns_per_div() * 1e-9
    }
    /// Label such as `"200us"` or `"1ms"`.
    pub fn label(self) -> String {
        let ns = self.ns_per_div() as u64;
        let (v, unit) = if ns < 1_000 {
            (ns, "ns")
        } else if ns < 1_000_000 {
            (ns / 1_000, "us")
        } else if ns < 1_000_000_000 {
            (ns / 1_000_000, "ms")
        } else {
            (ns / 1_000_000_000, "s")
        };
        format!("{v}{unit}")
    }
}

impl FromStr for TimeBase {
    type Err = Error;
    /// Accepts a label (`"200us"`) or a device index (`"15"`).
    fn from_str(s: &str) -> Result<Self> {
        for i in 0..Self::COUNT {
            if Self(i).label().eq_ignore_ascii_case(s) {
                return Ok(Self(i));
            }
        }
        s.parse::<u8>()
            .map_err(|_| Error::invalid(format!("unknown time base '{s}'")))
            .and_then(Self::from_index)
    }
}

/// Input coupling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coupling {
    Dc,
    Ac,
}

/// Trigger edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slope {
    Rising,
    Falling,
}

/// What to do when no trigger arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// Capture anyway after a short timeout (free running).
    Auto,
    /// Wait for a trigger; `capture` fails with a timeout if none arrives.
    Normal,
}

/// Per-channel vertical settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelConfig {
    pub enabled: bool,
    pub volts_per_div: VoltsPerDiv,
    pub coupling: Coupling,
    pub bandwidth_limit: bool,
    /// Vertical position, 0..=255. Zero volts is at ADC code 128 for the default 128 (see [`zero_code_for_position`]).
    pub position: u8,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            volts_per_div: VoltsPerDiv::V1,
            coupling: Coupling::Dc,
            bandwidth_limit: false,
            position: 128,
        }
    }
}

/// Pulse width comparison of the pulse trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulseCondition {
    /// Width equal to the setting (the hardware accepts +-5 %).
    Equal,
    NotEqual,
    /// Wider than the setting.
    Greater,
    /// Narrower than the setting.
    Less,
}

/// Analog television standard of the video trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoStandard {
    /// PAL and SECAM (625 lines).
    PalSecam,
    /// NTSC (525 lines).
    Ntsc,
}

/// What the video trigger synchronises to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSync {
    AllLines,
    /// One line number (1-based).
    Line(u16),
    OddField,
    EvenField,
    AllFields,
}

/// Kind of trigger.
///
/// Pulse and video triggers are implemented from the vendor code. The pulse trigger is verified on hardware with the
/// 1 kHz calibrator; the video trigger has not been verified against a video signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TriggerKind {
    /// Edge on the configured slope.
    Edge,
    /// Pulse width trigger; the slope selects positive (rising) or negative (falling) pulses.
    Pulse {
        /// Pulse width in nanoseconds, 10 ns to 10 s.
        width_ns: u64,
        condition: PulseCondition,
    },
    /// Video trigger.
    Video {
        standard: VideoStandard,
        sync: VideoSync,
        /// True for positive sync polarity.
        positive: bool,
    },
}

/// Trigger settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriggerConfig {
    /// Channel to trigger on (must be enabled).
    pub source: Channel,
    pub kind: TriggerKind,
    pub slope: Slope,
    pub sweep: Sweep,
    /// Trigger threshold as an ADC code (0..=228); it is compared with the raw sample values.
    pub level: u8,
    /// Position of the trigger inside the captured window, in percent (0..=100).
    pub horizontal_position_percent: f64,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            source: Channel::Ch1,
            kind: TriggerKind::Edge,
            slope: Slope::Rising,
            sweep: Sweep::Auto,
            level: 128,
            horizontal_position_percent: 50.0,
        }
    }
}

/// Complete acquisition setup.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub channels: [ChannelConfig; MAX_CHANNELS],
    pub timebase: TimeBase,
    pub trigger: TriggerConfig,
    /// Samples per channel in a frame. Only 4096 is verified on hardware.
    pub buffer_len: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channels: [ChannelConfig::default(); MAX_CHANNELS],
            timebase: TimeBase(15),
            trigger: TriggerConfig::default(),
            buffer_len: 4096,
        }
    }
}

impl Config {
    /// Bit mask of enabled channels (bit 0 = CH1).
    pub fn channel_mask(&self) -> u8 {
        self.channels
            .iter()
            .enumerate()
            .fold(0, |m, (i, c)| if c.enabled { m | 1 << i } else { m })
    }
    /// Number of enabled channels.
    pub fn enabled_count(&self) -> usize {
        self.channels.iter().filter(|c| c.enabled).count()
    }
    /// Check for obviously invalid combinations.
    pub fn validate(&self) -> Result<()> {
        if self.enabled_count() == 0 {
            return Err(Error::invalid("at least one channel must be enabled"));
        }
        if !self.channels[self.trigger.source.index()].enabled {
            return Err(Error::invalid("trigger source channel is not enabled"));
        }
        if self.trigger.level > 0xE4 {
            return Err(Error::invalid("trigger level must be 0..=228"));
        }
        match self.trigger.kind {
            TriggerKind::Edge => {}
            TriggerKind::Pulse { width_ns, .. } => {
                if !(10..=10_000_000_000).contains(&width_ns) {
                    return Err(Error::invalid("pulse width must be 10 ns to 10 s"));
                }
            }
            TriggerKind::Video { standard, sync, .. } => {
                if let VideoSync::Line(n) = sync {
                    let max = if standard == VideoStandard::PalSecam { 625 } else { 525 };
                    if !(1..=max).contains(&n) {
                        return Err(Error::invalid(format!("video line must be 1..={max}")));
                    }
                }
            }
        }
        if !(0.0..=100.0).contains(&self.trigger.horizontal_position_percent) {
            return Err(Error::invalid("horizontal position must be 0..=100 %"));
        }
        if self.buffer_len == 0 || self.buffer_len > 16384 {
            return Err(Error::invalid("buffer_len must be 1..=16384 samples"));
        }
        Ok(())
    }
}

/// Identification data read from the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeInfo {
    /// Raw 71-byte info string (control request `0xA2`, wValue `0x1580`).
    pub raw_info: Vec<u8>,
    /// Device family string (e.g. `"6000"`).
    pub name: String,
    /// PCB revision string (e.g. `"P1.06"`).
    pub pcb_version: String,
    /// Serial-like field from the info string.
    pub serial: String,
    /// FPGA version word returned by the reset command.
    pub fpga_version: u16,
}

/// ADC counts per vertical division in ADC channel mode `k` (1, 2 or 4; see `protocol::channel_mode`).
///
/// Measured on one 6254BC against a 0.994 V DC source (multimeter), 200 mV to 1 V per division, with the factory
/// gain trim applied: 24.15 in four channel mode, about 21.6 in two channel mode and about 21.1 in one channel
/// mode (the fast timebases). The vendor SDK demos assume 255/8 = 32 for all. Not verified on other units, below
/// 200 mV or above 1 V per division.
pub fn counts_per_div(k: u8) -> f64 {
    match k {
        1 => 21.1,
        2 => 21.6,
        _ => 24.15,
    }
}

/// ADC code of 0 V for a vertical position `0..=255` in channel mode `k`, as measured on a 6254BC with shorted input.
///
/// Position 128 is code 128 in every mode. The position control spans 8 divisions, so at position 0 and 255 the code is
/// about 31 and 224 in four channel mode, 39 and 216 in two channel mode, and 43 and 212 in one channel mode. The
/// ranges 50 mV, 500 mV and 5 V per division (indexes 4, 7 and 10) have a narrower span: 89 and 166, 92 and 162, and
/// 99 and 156.
pub fn zero_code_for_position(position: u8, volts_per_div: VoltsPerDiv, k: u8) -> f64 {
    let narrow = matches!(volts_per_div.index(), 4 | 7 | 10);
    let (lo, hi) = match (k, narrow) {
        (1, false) => (43.0, 212.0),
        (1, true) => (99.0, 156.0),
        (2, false) => (39.0, 216.0),
        (2, true) => (92.0, 162.0),
        (_, false) => (31.0, 224.0),
        (_, true) => (89.0, 166.0),
    };
    lo + f64::from(position) * (hi - lo) / 255.0
}

/// One acquisition.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Raw 8-bit samples per channel (empty for disabled channels).
    pub samples: [Vec<u8>; MAX_CHANNELS],
    /// Volts per division each channel was captured with.
    pub volts_per_div: [f64; MAX_CHANNELS],
    /// ADC code that corresponds to 0 V for each channel (see [`zero_code_for_position`]).
    pub zero_code: [f64; MAX_CHANNELS],
    /// ADC counts per division of this capture (see [`counts_per_div`]).
    pub counts_per_div: f64,
    /// Time between samples in nanoseconds.
    pub sample_interval_ns: f64,
    /// True if the hardware trigger fired.
    pub triggered: bool,
    /// Index of the trigger point within the samples, when triggered.
    pub trigger_sample: Option<usize>,
}

impl Frame {
    /// Samples of one channel, if it was enabled.
    pub fn channel(&self, ch: Channel) -> Option<&[u8]> {
        let s = &self.samples[ch.index()];
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    /// Convert one raw ADC code of `ch` to volts at the input (the probe attenuation is not applied).
    ///
    /// Uses `(code - zero) * V/div / counts_per_div`. The scale was measured on one 6254BC (see [`counts_per_div`]);
    /// the factory gain trim read from the device is applied by the driver, other per-unit differences are not.
    pub fn code_to_volts(&self, ch: Channel, code: u8) -> f64 {
        let i = ch.index();
        (f64::from(code) - self.zero_code[i]) * self.volts_per_div[i] / self.counts_per_div
    }

    /// Samples of one channel converted to volts, if it was enabled.
    pub fn volts(&self, ch: Channel) -> Option<Vec<f64>> {
        self.channel(ch)
            .map(|s| s.iter().map(|&c| self.code_to_volts(ch, c)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timebase_labels() {
        assert_eq!(TimeBase::from_index(0).unwrap().label(), "2ns");
        assert_eq!(TimeBase::from_index(15).unwrap().label(), "200us");
        assert_eq!(TimeBase::from_index(17).unwrap().label(), "1ms");
        assert_eq!(TimeBase::from_index(35).unwrap().label(), "1000s");
        assert_eq!("200us".parse::<TimeBase>().unwrap().index(), 15);
        assert!(TimeBase::from_index(36).is_err());
    }

    #[test]
    fn vdiv_parsing() {
        assert_eq!("1V".parse::<VoltsPerDiv>().unwrap().index(), 8);
        assert_eq!("200mv".parse::<VoltsPerDiv>().unwrap().volts(), 0.2);
        assert!("3V".parse::<VoltsPerDiv>().is_err());
    }

    #[test]
    fn config_validation() {
        let mut c = Config::default();
        assert!(c.validate().is_ok());
        c.channels[0].enabled = false;
        assert!(c.validate().is_err()); // trigger source disabled
    }
}

#[cfg(test)]
mod volts_tests {
    use super::*;

    #[test]
    fn code_to_volts_uses_measured_scale() {
        let frame = Frame {
            samples: [vec![128, 151, 105], vec![], vec![], vec![]],
            volts_per_div: [1.0, 1.0, 1.0, 1.0],
            zero_code: [128.0, 128.0, 128.0, 128.0],
            counts_per_div: 23.0,
            sample_interval_ns: 1.0,
            triggered: false,
            trigger_sample: None,
        };
        let v = frame.volts(Channel::Ch1).unwrap();
        assert_eq!(v[0], 0.0);
        assert!((v[1] - 1.0).abs() < 1e-12);
        assert!((v[2] + 1.0).abs() < 1e-12);
        let v1 = VoltsPerDiv::V1;
        assert!((zero_code_for_position(0, v1, 4) - 31.0).abs() < 1e-9);
        assert!((zero_code_for_position(255, v1, 4) - 224.0).abs() < 1e-9);
        assert!((zero_code_for_position(0, v1, 1) - 43.0).abs() < 1e-9);
        let v500m = VoltsPerDiv::from_index(7).unwrap();
        assert!((zero_code_for_position(0, v500m, 2) - 92.0).abs() < 1e-9);
        assert!((zero_code_for_position(255, v500m, 4) - 166.0).abs() < 1e-9);
        assert!(counts_per_div(1) < counts_per_div(2) && counts_per_div(2) < counts_per_div(4));
        assert!(frame.volts(Channel::Ch2).is_none());
    }
}
