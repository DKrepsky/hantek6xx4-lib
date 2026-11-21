# hantek6xx4-lib

Rust driver library for **Hantek 6xx4-series USB oscilloscopes**, developed against the Hantek 6254BC
(4 channels, 250 MHz, USB ID `04b5:6cde`). The protocol is reverse engineered; the
[protocol specification](https://github.com/DKrepsky/hantek6xx4-lib/blob/main/docs/protocol.md) states what is verified
on hardware. Not affiliated with or endorsed by Hantek.

- 1 to 4 channels, 12 vertical ranges, 36 time bases up to 1 GS/s, DC/AC coupling, bandwidth limit
- edge, pulse-width and video triggers
- raw samples or volts, with the device's factory gain calibration applied
- continuous scan/roll streaming
- connect/disconnect events (`DeviceMonitor`), with `Error::Disconnected` for a scope that is removed while in use
- a scriptable mock transport, so the driver can be tested without hardware
- never writes the device EEPROM or FPGA flash

```rust
use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope};

let mut scope = Scope::open()?;
let mut cfg = Config::default();
cfg.trigger.level = 128;
scope.configure(&cfg)?;
let frame = scope.capture(CaptureOptions::default())?;
let volts = frame.volts(Channel::Ch1).unwrap();
# Ok::<(), hantek6xx4_lib::Error>(())
```

Linux is the tested platform; there the user needs access to the USB device (udev rule in the repository,
`contrib/99-hantek6xx4.rules`). The crate uses the system `libusb-1.0`; enable the `vendored` feature to build it from
source instead. Windows and macOS are untested.
Minimum supported Rust version: 1.74. The companion command line tool is
[`hantek6xx4-cli`](https://crates.io/crates/hantek6xx4-cli).

License: MIT.
