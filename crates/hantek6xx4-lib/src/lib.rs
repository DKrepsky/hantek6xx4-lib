//! Driver library for Hantek 6xx4-series USB oscilloscopes.
//!
//! The first supported model is the Hantek 6254BC (4 channels, 250 MHz, USB ID `04b5:6cde`).
//! The protocol was reverse engineered; see `docs/protocol.md` in the repository for what is
//! verified on hardware and what is not.
//!
//! ```no_run
//! use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope};
//!
//! # fn main() -> hantek6xx4_lib::Result<()> {
//! let mut scope = Scope::open()?;
//! println!("{:?}", scope.info());
//! let mut cfg = Config::default();
//! cfg.trigger.level = 128;
//! scope.configure(&cfg)?;
//! let frame = scope.capture(CaptureOptions::default())?;
//! println!("CH1 first sample: {:?}", frame.channel(Channel::Ch1).map(|s| s[0]));
//! # Ok(()) }
//! ```
//!
//! Layers: [`protocol`] (pure wire format), [`transport`] (USB or mock), [`scope`] (high level API).

pub mod error;
pub mod protocol;
pub mod scope;
pub mod transport;
pub mod types;

pub use error::{Error, Result};
pub use scope::{CaptureOptions, Scope};
pub use transport::{MockTransport, Transport, UsbTransport};
pub use types::*;
