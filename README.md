# hantek6xx4-lib

A Rust driver library and command line tool for **Hantek 6xx4-series USB oscilloscopes**
(first target: **6254BC**, USB ID `04b5:6cde`). The protocol is not documented by the
vendor; it was reverse engineered and every behaviour used here was checked on a real
6254BC. See [`docs/protocol.md`](docs/protocol.md) for the protocol notes and for what is
verified and what is not.

> This project is not affiliated with or endorsed by Hantek.

## Status

Working on a 6254BC (see `docs/protocol.md` §9–§10):

- initialisation, identification, calibration table
- 1 / 2 / 4 channel acquisition at any of the 36 time bases, 12 vertical ranges
- DC/AC coupling bits and bandwidth limit (bandwidth limit verified; AC needs a signal with offset)
- edge trigger with level, slope, auto/normal sweep and horizontal position
- raw 8-bit samples (ADC codes). **No volts conversion yet** (scale not verified)

Not implemented / not verified: AWG, pulse/video triggers, roll/scan modes, other models
(6xx4BD should be the same hardware, untested), real sample rate of the 1 GS/s modes.

## Install access rights (Linux)

```sh
sudo cp contrib/99-hantek6xx4.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger   # then replug the scope
```
The user must be in the `plugdev` group. `libusb-1.0` development files are required to build.

## Command line

```sh
cargo run -p hantek6xx4-cli -- list
cargo run -p hantek6xx4-cli -- info
cargo run -p hantek6xx4-cli -- ranges
cargo run -p hantek6xx4-cli -- capture --channels 1,2 --vdiv 1V --timebase 200us
cargo run -p hantek6xx4-cli -- capture --normal --trigger-level 128 --format csv -o capture.csv
cargo run -p hantek6xx4-cli -- capture --channels 4 --normal --trigger-type pulse --pulse-width 500us --pulse-cond eq
cargo run -p hantek6xx4-cli -- raw bulk "06 00" --read 16      # protocol exploration
cargo run -p hantek6xx4-cli -- dump-calibration cal.bin         # read only
cargo run -p hantek6xx4-cli -- dump-amp-calibration amp.bin  # read only: factory gain trims (0xA2/0x1B00)
```
The binary is called `hantek6xx4`. Set `RUST_LOG=debug` for driver logging.

## Library

```rust
use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope};

let mut scope = Scope::open()?;
println!("{:?}", scope.info());

let mut cfg = Config::default();          // 4 channels, 1 V/div, 200 us/div, auto trigger
cfg.trigger.level = 128;                  // ADC code
scope.configure(&cfg)?;
let frame = scope.capture(CaptureOptions::default())?;
let ch1: &[u8] = frame.channel(Channel::Ch1).unwrap();
```

Layers: `protocol` (pure wire format, golden-byte tests) → `transport` (rusb backend and a scriptable
mock, so the whole driver is tested without hardware) → `scope` (high level API).

## Safety

The driver never writes the device EEPROM or the FPGA flash. Control requests `0xF0`/`0xF1`
(bitstream flash) and the `0xA2` write direction are deliberately not implemented: a bad write can
brick the scope.

## Development

```sh
cargo fmt --all && cargo clippy --all-targets && cargo test --all
```
`docs/research/` holds the throwaway probes and the Ghidra script used during reverse engineering.

## Acknowledgements

The open-source projects `libhantek6xx4` (BSD-2-Clause, Phil Vachon) and `pyhantek` (Daniel Kucera) and the
EEVblog 6254BD thread were the starting points for the reverse engineering. The code in this repository
is original.

## License

MIT, see [`LICENSE`](LICENSE).
