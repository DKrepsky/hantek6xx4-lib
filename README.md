# hantek6xx4-lib

[![CI](https://github.com/DKrepsky/hantek6xx4-lib/actions/workflows/ci.yml/badge.svg)](https://github.com/DKrepsky/hantek6xx4-lib/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Rust driver library and command line tool for **Hantek 6xx4-series USB oscilloscopes**, developed against the
**Hantek 6254BC** (4 channels, 250 MHz, USB ID `04b5:6cde`).

The vendor protocol is not documented. It was reverse engineered from the vendor SDK and checked on a real 6254BC;
the [protocol specification](docs/protocol.md) describes it and states for each feature whether it was verified on
hardware.

> This project is not affiliated with or endorsed by Hantek. "Hantek" is a trademark of its owner and is used here
> only to describe compatibility.

## Features

| Area | Support |
|---|---|
| Channels | 1, 2, 3 or 4 channels, 12 vertical ranges (2 mV to 10 V per division), DC/AC coupling, bandwidth limit |
| Time base | 36 settings, 2 ns to 1000 s per division, up to 1 GS/s |
| Triggers | edge, pulse width (greater, less, equal, not equal), video; auto and normal sweep; trigger position |
| Data | raw 8-bit samples or volts, with the factory gain calibration of the device applied |
| Streaming | continuous scan/roll acquisition |
| Tooling | CLI with CSV/raw export and calibration backup, scriptable mock transport for tests |

Known limits: the video trigger has not been tested with a video signal; volts are verified from 50 mV to 1 V per
division on one unit; time bases slower than 50 ms per division were tested at four settings only; the 6254BC has no
signal generator; serial-bus triggers (CAN, LIN, UART, SPI, I2C) are not implemented. Other models of the family (for
example the 6254BD) are untested. The driver never writes the device EEPROM or FPGA flash.

## Crates

| Crate | Purpose |
|---|---|
| [`hantek6xx4-lib`](crates/hantek6xx4-lib) | the driver library |
| [`hantek6xx4-cli`](crates/hantek6xx4-cli) | the `hantek6xx4` command line tool |

## Installation

Build requirements: a Rust toolchain (library 1.74 or newer, CLI 1.85 or newer). The `rusb` backend uses the system
`libusb-1.0`; on Linux install its development package (`libusb-1.0-0-dev` and `pkg-config` on Debian and Ubuntu).
Where no system libusb is available, for example on Windows, build with `--features vendored` to compile libusb from
source.

```sh
cargo install hantek6xx4-cli          # installs the `hantek6xx4` binary
cargo install hantek6xx4-cli --features vendored
```

### Access rights on Linux

```sh
sudo cp contrib/99-hantek6xx4.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger   # then replug the scope
```

The rule grants access to the `plugdev` group; make sure your user is a member.

## Command line

```sh
hantek6xx4 list                                        # connected scopes
hantek6xx4 info                                        # identification, versions, calibration status
hantek6xx4 ranges                                      # vertical ranges and time bases
hantek6xx4 capture --channels 1,2 --vdiv 1V --timebase 200us --volts
hantek6xx4 capture --normal --trigger-level 128 --format csv -o capture.csv
hantek6xx4 capture --channels 4 --normal --trigger-type pulse --pulse-width 500us --pulse-cond eq
hantek6xx4 roll --channels 2 --timebase 25 --duration 20 --format csv
hantek6xx4 dump-calibration cal.bin                    # read-only backups of the factory tables
hantek6xx4 dump-amp-calibration amp.bin
hantek6xx4 raw bulk "06 00" --read 16                  # low-level access for protocol work
```

Set `RUST_LOG=debug` for driver logging. `hantek6xx4 <command> --help` lists all options.

## Library

```rust
use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope};

let mut scope = Scope::open()?;

let mut cfg = Config::default();          // 4 channels, 1 V/div, 200 us/div, auto trigger
cfg.trigger.level = 128;                  // ADC code
scope.configure(&cfg)?;

let frame = scope.capture(CaptureOptions::default())?;
let volts: Vec<f64> = frame.volts(Channel::Ch1).unwrap();
println!("{} samples every {} ns", volts.len(), frame.sample_interval_ns);
# Ok::<(), hantek6xx4_lib::Error>(())
```

The driver is built in layers: `protocol` (the wire format as pure functions with golden-byte tests), `transport`
(the `rusb` backend and a scriptable mock, so the whole driver is tested without hardware) and `scope` (the high level
API). Runnable examples are in [`crates/hantek6xx4-lib/examples`](crates/hantek6xx4-lib/examples).

## Safety

The driver does not write the device EEPROM or the FPGA flash. The control requests that do (`0xF0`/`0xF1`, the
`0xA2` write direction and `0xEC`) are deliberately not implemented, because a bad write can leave the scope unusable.
The `dump-*` commands only read.

## Documentation

- [Protocol specification](docs/protocol.md): transport, commands, configuration model, calibration, verification status
- [Research notes](docs/research/README.md): how the protocol was obtained and the probes used
- [Changelog](CHANGELOG.md), [release checklist](RELEASING.md)

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

Hardware is not needed for the tests. Contributions that add support for other models or verify untested features on
other units are welcome; please include captures or measurements with the pull request.

## Acknowledgements

The open-source projects `libhantek6xx4` (BSD-2-Clause) and `pyhantek` and the EEVblog 6254BD thread were the
starting points of the reverse engineering. The code in this repository is original.

## License

MIT, see [`LICENSE`](LICENSE).
