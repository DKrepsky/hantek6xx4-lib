//! Error type for the whole crate.

use std::fmt;

/// Everything that can go wrong while talking to the scope.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Low-level USB failure (permissions, unplugged device, stall, ...).
    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),
    /// No supported device is connected.
    #[error("no Hantek 6xx4 device found (is it plugged in and is the udev rule installed?)")]
    NotFound,
    /// A parameter given by the caller is out of range or unsupported.
    #[error("invalid parameter: {0}")]
    InvalidParameter(String),
    /// The device answered with data we do not understand.
    #[error("unexpected device response: {0}")]
    Protocol(String),
    /// The device did not become ready in time.
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
    /// The calibration table stored in the device failed its validity check.
    #[error("calibration table in the device is invalid")]
    BadCalibration,
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn invalid(msg: impl fmt::Display) -> Self {
        Error::InvalidParameter(msg.to_string())
    }
}
