//! Abstraction over the USB link so the protocol code can be tested without hardware.

use crate::error::Result;

mod mock;
mod usb;

pub use mock::{Call, MockTransport};
pub use usb::{DeviceInfo, UsbTransport, PID, VID};

/// Minimal set of USB operations the scope needs (vendor control requests plus two bulk pipes).
pub trait Transport {
    /// Vendor control transfer, device to host.
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>>;
    /// Vendor control transfer, host to device.
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()>;
    /// Bulk write to endpoint `0x02`.
    fn bulk_out(&mut self, data: &[u8]) -> Result<()>;
    /// Bulk read from endpoint `0x86` of up to `len` bytes.
    fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>>;
    /// Wait `ms` milliseconds (mocks override this to avoid slow tests).
    fn delay(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
