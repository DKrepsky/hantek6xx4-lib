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

/// Where a scope is attached to the USB bus. Bus and address identify one physical connection until the device is
/// unplugged: a device that is plugged in again usually gets a new address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceInfo {
    /// USB bus number.
    pub bus: u8,
    /// Device address on the bus.
    pub address: u8,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bus {} address {} ({:04x}:{:04x})",
            self.bus, self.address, self.vendor_id, self.product_id
        )
    }
}

impl DeviceInfo {
    pub(crate) fn of<T: UsbContext>(device: &rusb::Device<T>) -> Option<Self> {
        let d = device.device_descriptor().ok()?;
        (d.vendor_id() == VID && d.product_id() == PID).then(|| Self {
            bus: device.bus_number(),
            address: device.address(),
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
        })
    }
}

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
            .filter(|d| DeviceInfo::of(d).is_some())
            .nth(n)
            .ok_or(Error::NotFound)?;
        Self::claim(dev)
    }

    /// Open the scope at a given bus position, for example the one reported by a
    /// [`DeviceEvent::Connected`](crate::DeviceEvent::Connected). Fails with [`Error::NotFound`] if it is gone.
    pub fn open_device(info: DeviceInfo) -> Result<Self> {
        let dev = GlobalContext::default()
            .devices()?
            .iter()
            .find(|d| DeviceInfo::of(d) == Some(info))
            .ok_or(Error::NotFound)?;
        Self::claim(dev)
    }

    fn claim(dev: rusb::Device<GlobalContext>) -> Result<Self> {
        let handle = dev.open()?;
        handle.claim_interface(0)?;
        Ok(Self { handle })
    }

    /// List `(bus, address)` of all connected scopes.
    pub fn enumerate() -> Result<Vec<(u8, u8)>> {
        Ok(Self::devices()?.into_iter().map(|d| (d.bus, d.address)).collect())
    }

    /// All connected scopes, in bus order.
    pub fn devices() -> Result<Vec<DeviceInfo>> {
        Ok(GlobalContext::default()
            .devices()?
            .iter()
            .filter_map(|d| DeviceInfo::of(&d))
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
