# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/) (while below 1.0, minor versions may change the API).

## [Unreleased]

## [0.2.0](https://github.com/DKrepsky/hantek6xx4-lib/compare/v0.1.0...v0.2.0) - 2026-11-21

### Added

- [**breaking**] add DeviceMonitor for connect/disconnect events and Error::Disconnected

### Fixed

- [**breaking**] mark Error as non_exhaustive

## [0.1.0]

First release. Developed and verified on a Hantek 6254BC (USB `04b5:6cde`, PCB P1.06, FPGA `0xd001`).

### Added

- `hantek6xx4-lib`: `Scope` API (`open`, `info`, `configure`, `capture`, `start_roll`/`poll_roll`, `stop`), typed
  configuration, conversion to volts, `Transport` trait with `rusb` and mock backends, the wire format as pure
  functions with golden-byte tests
- 1, 2, 3 and 4 channel acquisition at all 36 time bases and 12 vertical ranges, DC/AC coupling, bandwidth limit
- edge, pulse-width and video triggers (video not verified with a video signal), auto and normal sweep, trigger position
- application of the factory gain trim table (`0xA2/0x1B00`) and the position calibration table (`0xA2/0x1600`)
- scan/roll streaming from the circular capture RAM
- `hantek6xx4-cli`: `list`, `info`, `state`, `ranges`, `capture`, `roll`, `dump-calibration`, `dump-amp-calibration`,
  `raw`
- `vendored` feature to build libusb from source
- protocol specification in `docs/protocol.md`

[Unreleased]: https://github.com/DKrepsky/hantek6xx4-lib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/DKrepsky/hantek6xx4-lib/releases/tag/v0.1.0
