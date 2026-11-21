//! Error type for the whole crate.

use std::fmt;

/// Everything that can go wrong while talking to the scope.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Low-level USB failure (permissions, stall, ...). An unplugged device is reported as [`Error::Disconnected`].
    #[error("USB error: {0}")]
    Usb(rusb::Error),
    /// The device was unplugged (or lost power) while it was in use.
    #[error("the device was disconnected")]
    Disconnected,
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
    /// Scan/roll data was probably lost because the capture RAM wrapped before it was read.
    #[error("roll buffer overrun: the scope was polled too slowly, samples were lost")]
    Overrun,
    /// The calibration table stored in the device failed its validity check.
    #[error("calibration table in the device is invalid")]
    BadCalibration,
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

impl From<rusb::Error> for Error {
    fn from(e: rusb::Error) -> Self {
        match e {
            rusb::Error::NoDevice => Error::Disconnected,
            e => Error::Usb(e),
        }
    }
}

impl Error {
    /// True if the error means the device is gone, so the application should drop its [`Scope`](crate::Scope) and wait
    /// for the device to come back (see [`DeviceMonitor`](crate::DeviceMonitor)).
    pub fn is_disconnected(&self) -> bool {
        matches!(self, Error::Disconnected)
    }

    pub(crate) fn invalid(msg: impl fmt::Display) -> Self {
        Error::InvalidParameter(msg.to_string())
    }
}
