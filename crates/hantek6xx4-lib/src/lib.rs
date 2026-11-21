//! Driver library for Hantek 6xx4-series USB oscilloscopes.
//!
//! The first supported model is the Hantek 6254BC (4 channels, 250 MHz, USB ID `04b5:6cde`). The protocol is not
//! documented by the vendor; it was reverse engineered, and the
//! [protocol specification](https://github.com/DKrepsky/hantek6xx4-lib/blob/main/docs/protocol.md) says for every
//! feature whether it was verified on hardware.
//!
//! This project is not affiliated with or endorsed by Hantek.
//!
//! # Capabilities
//!
//! - 1, 2, 3 or 4 channel acquisition at all 36 time bases (up to 1 GS/s) and 12 vertical ranges, DC/AC coupling and
//!   bandwidth limit, 4096 samples per channel and frame
//! - edge, pulse-width and video triggers, auto and normal sweep, trigger position
//! - raw 8-bit samples and conversion to volts ([`Frame::volts`]), with the per-unit factory calibration of the
//!   device applied
//! - continuous scan/roll streaming ([`Scope::start_roll`], [`Scope::poll_roll`])
//!
//! Not implemented: the arbitrary waveform generator (the 6254BC has none), serial-bus triggers (CAN, LIN, UART, SPI,
//! I2C), and any write to the device EEPROM or FPGA flash.
//!
//! # Example
//!
//! ```no_run
//! use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope, TriggerKind, PulseCondition};
//!
//! # fn main() -> hantek6xx4_lib::Result<()> {
//! let mut scope = Scope::open()?;
//! println!("{:?}", scope.info());
//!
//! // CH1 only, 1 V/div, 200 us/div, edge trigger at an ADC code of 128
//! let mut cfg = Config::default();
//! for (i, ch) in cfg.channels.iter_mut().enumerate() {
//!     ch.enabled = i == 0;
//! }
//! cfg.trigger.level = 128;
//! scope.configure(&cfg)?;
//!
//! let frame = scope.capture(CaptureOptions::default())?;
//! let volts = frame.volts(Channel::Ch1).expect("CH1 is enabled");
//! println!("{} samples, first {:.3} V, interval {} ns", volts.len(), volts[0], frame.sample_interval_ns);
//!
//! // pulse-width trigger: positive pulses of about 500 us
//! cfg.trigger.kind = TriggerKind::Pulse { width_ns: 500_000, condition: PulseCondition::Equal };
//! # Ok(()) }
//! ```
//!
//! Without hardware, the whole driver can run on a scripted [`MockTransport`]; the integration tests of this crate
//! do that.
//!
//! # Layers
//!
//! - [`protocol`]: the wire format as pure functions (bytes in, bytes out), covered by golden-byte tests
//! - [`transport`]: the [`Transport`] trait with a `rusb` backend ([`UsbTransport`]) and the mock
//! - [`scope`]: the high level [`Scope`] API
//!
//! # Platform notes
//!
//! Linux is the tested platform. The user needs access to the USB device there (a udev rule is in `contrib/` in the
//! repository). The `rusb` backend uses the system `libusb-1.0`; enable the `vendored` feature to build libusb from
//! source instead. Windows and macOS are untested.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod error;
pub mod protocol;
pub mod scope;
pub mod transport;
pub mod types;

pub use error::{Error, Result};
pub use scope::{CaptureOptions, Scope};
pub use transport::{MockTransport, Transport, UsbTransport};
pub use types::*;
