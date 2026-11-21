# Research notes

How the protocol in [`../protocol.md`](../protocol.md) was obtained, which sources were used and what the research
tools are. The specification itself is the reference; this document explains its provenance.

## Method

The vendor does not document the protocol and no Windows machine was available, so the work followed this sequence:

1. **Public sources.** Existing open-source drivers and forum teardowns gave the transport (USB ID, endpoints, vendor
   control requests) and a partial command set.
2. **Decompiling the vendor SDK.** The Windows SDK ships `HTHardDll.dll`, which builds every command. It was decompiled
   with Ghidra (headless, using `ExportDecomp.java` in this directory) and read together with the SDK headers, the manual
   and the demo applications. Floating-point constants, which the decompiler drops, were read from the `.rdata` section
   with `objdump`, and x87 code was read from the disassembly.
3. **Replay on the device.** Throwaway Rust probes (`replay_probe`, `config_probe`) replayed the decoded commands on a
   real 6254BC, first read-only, then the configuration commands, then captures of known signals.
4. **Measurement against known references.** The 1 kHz probe-compensation output, DC sources read with a multimeter
   and shorted inputs were used to check timing, amplitude, triggers and the calibration model. Where the vendor code
   and the hardware disagreed, the hardware won (pulse width unit, counts per division, zero level).

The vendor SDK is not redistributed here. To repeat the decompilation, obtain the SDK from the manufacturer,
install Ghidra and run, for example:

```sh
analyzeHeadless <project dir> <project name> -import HTHardDll.dll -postScript ExportDecomp.java <output.c>
```

## Where the vendor code and public sources were wrong or incomplete

| Topic | Public source or vendor documentation | What the 6254BC does |
|---|---|---|
| Horizontal trigger position (`10`) | two hard-coded values in both open-source drivers | computed from window length, position, sample interval and FPGA latency (specification 7.4); the hard-coded values are the 50 % case at 800 ns |
| Trigger level (`07`) | formula unexplained | the level is an ADC code, with `trunc(nPos × 200/256 + 28.5)` in the vendor API |
| Relay update | "latch 0 then 1" | the register image is sent twice (full, then `AND 0x86`) with 4 ms and 50 ms pauses |
| Channel position | taken from a captured trace | computed from the `0x1600` calibration table |
| Counts per division | 32 (255 / 8) in the SDK demos | 24.15 / 21.6 / 21.1 by ADC channel mode |
| Zero level | `255 − position` in the SDK demos | `lo + position × (hi − lo) / 255` with mode and range dependent ends |
| Pulse width unit | 5 ns in the vendor header | the argument is effectively nanoseconds; the counter runs at 125 MHz |
| `0xEA` bus select | first step of the vendor initialisation | stalls on the 6254BC; everything works without it |
| `0x1E` command | channel offsets in pyhantek | it is the gain-trim command of the vendor library (specification 7.7) |
| Info string serial | offsets `[20..27]` in libhantek6xx4 | the vendor library uses `[34..42)`; `[20..27]` lands on `D1.00` |
| Generator | listed as present | the 6254BC unit has no generator output and blank calibration areas |
| `0x13` pulse trigger | defined, not implemented in either project | decoded and verified |

## Related projects

| Project | Value | License |
|---|---|---|
| [pvachon/libhantek6xx4](https://github.com/pvachon/libhantek6xx4) | Best protocol reference (C, libusb): startup, SPI map, front-end bits, status | BSD-2-Clause |
| [danielkucera/pyhantek](https://github.com/danielkucera/pyhantek) | Only working end-to-end capture in the public projects (Python), plus a scopehal bridge; many magic numbers | none stated (reference only) |
| [hackhantek/hantek6004](https://github.com/hackhantek/hantek6004) | Early work from the Android library; abandoned in 2018 | unverified |
| [SiberK/HANTEK_DSO_6254BD](https://github.com/SiberK/HANTEK_DSO_6254BD) | Windows application around the vendor DLLs; shows call order and tables | none stated |
| [sigrok Hantek 6254BD](https://sigrok.org/wiki/Hantek_6254BD) | BOM and `lsusb` output; status "planned" | n/a |

`pyhantek` and the SiberK application state no license and were used as references only. No code was copied from any of these projects; this repository's code is original.

### Protocols that look similar but are not the same

- **OpenHantek, OpenHantek6022, sigrok `hantek-6xxx`, Hantek6022API**: the 6022 family. Different protocol (vendor requests
  `0xE0` to `0xE5`, firmware uploaded by the host, other vendor IDs).
- **mikrocontroller.net "Hantek Protokoll"**: the DSO5xxxB and DST1xxxB family, a framed protocol with a checksum. It does
  not cover the 6xxx series.

## Sources

- sigrok wiki: [Hantek 6254BD](https://sigrok.org/wiki/Hantek_6254BD) and its [Info](https://sigrok.org/wiki/Hantek_6254BD/Info) page
- EEVblog forum: [Hantek 6254BD 250MHz 1GSa/s PC/USB DSO](https://eevblog.com/forum/testgear/hantek-6254bd-250mhz-1gsas-pcusb-dso)
- Hantek SDK for the 6000B series (headers, manual and demos), distributed by the manufacturer
- [libsigrok hantek-6xxx](https://github.com/sigrokproject/libsigrok/tree/master/src/hardware/hantek-6xxx),
  [Hantek6022API](https://github.com/Ho-Ro/Hantek6022API), [mikrocontroller.net](https://www.mikrocontroller.net/articles/Hantek_Protokoll)

## Tools in this directory

| Path | Purpose |
|---|---|
| `ExportDecomp.java` | Ghidra script that writes the decompiled functions of the loaded program to a C file |
| `replay_probe/` | Replays the transport, initialisation and a first capture with direct libusb calls |
| `config_probe/` | Implements the configuration model (ADC modes, relays, trigger, positions) and the shorted-input sweeps |

The probes are throwaway research code, kept for reference, and are excluded from the workspace. They predate the
library and are not maintained; the library and its tests are the supported implementation.

## Risks recorded during the research

- A 6204BD was bricked by flashing OpenHantek 6022 firmware, and a 6104BC was bricked and recovered by writing the EEPROM
  image of a 6074BC. Back up the EEPROM before any write; this project never writes it.
- Windows driver signing affects only the official software; on Linux a udev rule is enough.
- The USB ID the official software opens is reported inconsistently (`04b4` and `04b5`); `04b5:6cde` was confirmed on the
  6254BC.
