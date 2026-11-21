# hantek6xx4-cli

Command line tool for **Hantek 6xx4-series USB oscilloscopes** (developed against the 6254BC), built on
[`hantek6xx4-lib`](https://crates.io/crates/hantek6xx4-lib). Not affiliated with or endorsed by Hantek.

```sh
cargo install hantek6xx4-cli           # installs the `hantek6xx4` binary
hantek6xx4 list
hantek6xx4 info
hantek6xx4 capture --channels 1,2 --vdiv 1V --timebase 200us --volts
hantek6xx4 capture --normal --trigger-level 128 --format csv -o capture.csv
hantek6xx4 capture --channels 4 --normal --trigger-type pulse --pulse-width 500us --pulse-cond eq
hantek6xx4 roll --channels 2 --timebase 25 --duration 20 --format csv
hantek6xx4 watch                         # print a line whenever a scope is plugged in or removed
hantek6xx4 dump-calibration cal.bin      # read-only backups of the factory tables
hantek6xx4 dump-amp-calibration amp.bin
```

`hantek6xx4 <command> --help` lists all options. Build requirements, Linux access rights (udev rule) and the protocol
specification are described in the
[repository README](https://github.com/DKrepsky/hantek6xx4-lib#readme). Linux is the tested platform. Minimum supported
Rust version: 1.85; use `--features vendored` where no system libusb is available.

License: MIT.
