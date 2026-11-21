//! `rusb` (libusb) backend.

use super::Transport;
use crate::error::{Error, Result};
use rusb::{request_type, DeviceHandle, Direction, GlobalContext, Recipient, RequestType, UsbContext};
use std::time::Duration;

/// USB vendor id of the 6xx4 series.
pub const VID: u16 = 0x04b5;
/// USB product id of the 6xx4 series.
pub const PID: u16 = 0x6cde;

const EP_OUT: u8 = 0x02;
const EP_IN: u8 = 0x86;
const TIMEOUT: Duration = Duration::from_millis(2000);

/// A claimed USB interface of a connected scope.
pub struct UsbTransport {
    handle: DeviceHandle<GlobalContext>,
}

impl UsbTransport {
    /// Open the first connected scope.
    pub fn open() -> Result<Self> {
        Self::open_nth(0)
    }

    /// Open the `n`-th connected scope (0 based, in bus order).
    pub fn open_nth(n: usize) -> Result<Self> {
        let dev = GlobalContext::default()
            .devices()?
            .iter()
            .filter(|d| {
                d.device_descriptor()
                    .map(|dd| dd.vendor_id() == VID && dd.product_id() == PID)
                    .unwrap_or(false)
            })
            .nth(n)
            .ok_or(Error::NotFound)?;
        let handle = dev.open()?;
        handle.claim_interface(0)?;
        Ok(Self { handle })
    }

    /// List `(bus, address)` of all connected scopes.
    pub fn enumerate() -> Result<Vec<(u8, u8)>> {
        Ok(GlobalContext::default()
            .devices()?
            .iter()
            .filter(|d| {
                d.device_descriptor()
                    .map(|dd| dd.vendor_id() == VID && dd.product_id() == PID)
                    .unwrap_or(false)
            })
            .map(|d| (d.bus_number(), d.address()))
            .collect())
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}

impl Transport for UsbTransport {
    fn control_in(&mut self, request: u8, value: u16, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        let rt = request_type(Direction::In, RequestType::Vendor, Recipient::Device);
        let n = self.handle.read_control(rt, request, value, 0, &mut buf, TIMEOUT)?;
        buf.truncate(n);
        Ok(buf)
    }

    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        let rt = request_type(Direction::Out, RequestType::Vendor, Recipient::Device);
        self.handle.write_control(rt, request, value, 0, data, TIMEOUT)?;
        Ok(())
    }

    fn bulk_out(&mut self, data: &[u8]) -> Result<()> {
        let n = self.handle.write_bulk(EP_OUT, data, TIMEOUT)?;
        if n != data.len() {
            return Err(Error::Protocol(format!(
                "short bulk write: {n} of {} bytes",
                data.len()
            )));
        }
        Ok(())
    }

    fn bulk_in(&mut self, len: usize) -> Result<Vec<u8>> {
        // libusb needs a multiple of the 512-byte max packet size or a short buffer overflows
        let mut buf = vec![0u8; len.div_ceil(512).max(1) * 512];
        let n = self.handle.read_bulk(EP_IN, &mut buf, TIMEOUT)?;
        buf.truncate(n.min(len));
        Ok(buf)
    }
}
