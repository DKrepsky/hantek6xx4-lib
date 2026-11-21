# Hantek 6254BC — Protocol Research

Status: **research notes, unverified against hardware.** Compiled 2026-11-20 from public sources
(forum posts, GitHub repos, sigrok wiki). Byte values were extracted from those sources and have
**not** been tested on a real 6254BC. Almost all public material targets the 6254**BD**; forum
posters say the 6xx4BC and 6xx4BD share the same PCB and protocol (differences are believed to be
in EEPROM/FPGA firmware), but this is opinion until confirmed on our unit.

## 1. Summary

- No complete public protocol spec or finished open-source driver exists for the 6254BC.
- Two partial reverse-engineering efforts exist for the 6xx4 family (libhantek6xx4, pyhantek) and
  agree on the transport and most of the command set.
- A driver is feasible, but we must verify and finish the protocol ourselves (USB traces of the
  official software).

## 1a. Verified on our 6254BC unit (2026-11-20)

Read-only probe (Rust + rusb, only control reads, no writes):

- `lsusb`: `04b5:6cde` "ROHM LSI Systems USA / DSO Device", bcdUSB 2.00 (high speed), bcdDevice 0.00,
  iSerial 0. Interface 0 vendor class, EP `0x02` OUT bulk 512, EP `0x86` IN bulk 512.
  **Confirms the USB ID and endpoints for the BC.** Device is already running firmware (no loader).
- Control `0xB2` (IN, 10-byte buffer): returned **1 byte**, `01` = ready.
- Control `0xA2` wValue `0x1580` (IN, 71 bytes), `0xFF` bytes are separators/padding:
  ```
  44 53 4f ff ff ff 36 30 30 30 ff ff ff ff 50 31   DSO...6000....P1
  2e 30 36 ff 44 31 2e 30 30 ff 4d 30 30 37 43 30   .06.D1.00.M007C0
  31 31 36 34 43 30 32 31 35 33 32 30 31 37 30 32   1164C02153201702
  32 32 32 30 31 37 30 32 32 32 54 30 33 33 46 32   2220170222T033F2
  c0 30 31 ff 03 ff ff                              .01....
  ```
  Fields: `DSO`, `6000` (family), `P1.06` (PCB revision 1.06), `D1.00`, then an `M…` blob
  (`M007C01164C0215320170222201702 22…T033F2…01`) that likely holds serial/date/model codes.
  The string does **not** name "6254BC"; model identification needs the decompiled software or
  EEPROM comparison. Note libhantek6xx4's offsets (PCB rev at `[14..18]`) match `P1.06`; its
  serial offset `[20..27]` lands on `D1.00\xffM0`, so that offset is probably wrong.
- Control `0xA2` wValue `0x15E0` (IN, 8 bytes): `08 bf ff ff ff ff ea ea`.
- Control `0xA2` wValue `0x1600` (IN, 1154 bytes = 577 u16): **valid**, last u16 = `0xFBCF`.
  Mostly default values `0x0004` / `0x0005`, with non-default groups of 8 u16 at the start of
  some blocks (per-channel/per-range calibration), layout still to be decoded.
- Not yet tried: any bulk command (`09 00`, `0C 00`, status, capture).

## 2. Hardware (6254BD teardown, assumed same for BC)

| Part | Chip / detail |
|---|---|
| USB controller | Cypress CY7C68013A (FX2LP, 8051). Firmware loaded from EEPROM at power-up |
| EEPROM | Microchip 24LC64 (64 Kbit I2C), holds FX2 firmware |
| FPGA | Xilinx Spartan-6 XC6SLX16 |
| FPGA config flash | Macronix MX25L4006E (4 Mbit SPI), wired to FX2 port D pins (PD0–PD4) |
| ADC | Analog Devices HMCAD1511, 8-bit, 1 GS/s, 4 channels |
| PLL / clock | Analog Devices ADF4360-7 |
| Front end control | 74HC595 shift registers (one per channel): attenuator relays, AC/DC relay, bandwidth limit, ADC driver enable |
| AWG DAC | TI DAC902E (12-bit) |
| Capture memory | 64K |
| Other | 24 MHz crystal, TLV274C op-amps, TPS63000 buck-boost (U38) |

Spec (6254BD): 4 channels, 250 MHz, 1 GSa/s, 8-bit, 1 MΩ‖25 pF, edge/pulse/video/alternate triggers.

Forum claims (unverified): there is no hardware difference between 6xx4 models, bandwidth is
limited in software/EEPROM/FPGA config; BC and BD PCBs look the same with small component
population differences.

## 3. USB transport

- **USB ID:** `04b5:6cde` ("ROHM LSI Systems USA" in lsusb; iManufacturer "ODM", iProduct "DSO Device").
  Seen on 6254BD and 6204BD. A Hantek support post says the Windows INF uses the same ID while the
  scope reports `04b4:8613` in software. **The 6254BC ID is unconfirmed.**
- Class 0xFF (vendor specific), 1 configuration, 1 interface, bus powered (500 mA).
- **No firmware upload at connect:** a Windows capture showed only standard enumeration plus
  control/bulk traffic, so the FX2 already runs its EEPROM firmware. (Unlike the 6022, which needs
  firmware uploaded by the host.)
- **Endpoints:** bulk OUT `0x02` (commands), bulk IN `0x86` (responses and samples), wMaxPacketSize 512.
  If the device is not in high-speed mode, packets are 64 bytes.

### 3.1 Vendor control requests (wIndex = 0)

| bRequest | Dir | Purpose |
|---|---|---|
| `0xB3` (179) | OUT | Initialize/reset. Sent before each bulk command with 10-byte preamble `0f 03 03 03 00 00 00 00 00 00` |
| `0xB2` (178) | IN, 10 B | Check ready. Byte 0 == 1 means ready (also selects 512 vs 64 byte packets) |
| `0xA2` (162) | IN | Device info. wValue `0x1580`, len 0x47: ID string (PCB revision at id[14..18], serial at id[20..27]). wValue `0x1600`: calibration data, 12*12*4+1 = 577 uint16, last entry `0xFBCF` = valid flag. wValue `0x15E0`, len 8: second info read (pyhantek, "driver version?") |
| `0xF1` (241) | — | Bitstream flash access, wValue `0x1E00` (defined in libhantek6xx4 `hantek_flash.c`, not reviewed) |
| `0xEA` (234) | OUT | 10 zero bytes, early setup step in pyhantek (purpose unknown) |

### 3.2 Bulk OUT command set (endpoint 0x02)

Consistent between libhantek6xx4 and pyhantek. First byte is the command ID.

| Cmd | Bytes | Purpose |
|---|---|---|
| `0x0C` | `0C 00` | Reset |
| `0x09` | `09 00`, then 4-byte read | Hardware (FPGA) version |
| `0x08` | `08 00 00 lo hi reg cs 00` | SPI write. `cs`: `0x1` front-end shift reg, `0x2` ADF4360, `0x4` HMCAD1511 |
| `0x0F` | `0F 00 <u32 LE>` | Timebase / sample-rate divider. pyhantek: `raw = (250e6 / rate)/2 - 1` (max 250 MHz via this path); libhantek6xx4 sends `table value - 1` |
| `0x00,0x01,0x02,0x04` | `id 00 lo hi` | Channel vertical position for CH1..CH4 (note CH4 = `0x04`), derived from calibration data |
| `0x07` | `07 00` + 24 bytes | Trigger level: 4 groups of (high,high,low,low) + 8 copies of the level. Level capped at `0xE4` |
| `0x11` | `11 00 mode slope coupling 00` | Trigger config. mode: edge 0, pulse 1, video 2, force 0x80. slope: rise 0, fall 1. coupling: DC 0, AC 1 |
| `0x12` | `12 00 b2 00 00 b5` | Trigger source. libhantek6xx4: `b2 = ((peak&1)<<6)\|(chanmask<<2)\|mask`, `b5 = (!enabled<<2)\|ch`. Marked FIXME (valid only at ≤250 MSa/s) |
| `0x10` | 14 bytes | Horizontal trigger position. **Hard-coded in both projects** (`0x831C4`, `0x7D7D0` / `214828`, `205600`) |
| `0x13` | — | Special trigger (pulse/video). Defined, not implemented anywhere |
| `0x03` | `03 00 mode 00` | Start capture. mode: auto 0, roll 1, single 2 (pyhantek uses `03 00 01 00`) |
| `0x06` | `06 00`, then 1-byte read | Status: bit0 triggered, bit1 data ready, bit3 pack state, bit4 SDRAM init |
| `0x0D` | `0D 00`, then 5-byte read | Buffer status (40-bit word; pyhantek: `trig23 = d[2]+d[3]*256`, `trig45 = d[4]`) |
| `0x0E` | `0E 00 lo hi` | Buffer prepare transfer. Defined; pyhantek sends it with 0 |
| `0x05` | `05 00 lo hi` | Read back buffer. Length field = buffer bytes / 2 per libhantek6xx4; pyhantek sends `05 00 00 80` (128 packets × 512 = 64 KiB) |
| `0x1E` | `1E 00 00 04 00 04 00 04 00 04 00` + 16 zero bytes | Channel offsets (pyhantek, `config_channel_offset`) |

### 3.3 Front-end shift register (SPI cs=0x1)

Message: `08 00 00 ch0 ch1 ch2 ch3 latch`, sent with latch = 0 then 1. Per-channel byte bits:

| Bit | Meaning |
|---|---|
| 7 | Bandwidth limit |
| 6 | Range > 1 V |
| 5 | Range ≤ 1 V |
| 4 | Range > 100 mV |
| 3 | Range ≤ 100 mV |
| 2 | Coupling (1 = DC) |
| 1 | Always set |

HMCAD1511 coarse-gain code per V/div (12 settings, 2 mV … 10 V, 1-2-5):
`{0xD,0xA,0x7,0x5,0x2,0x0,0x5,0x2,0x0,0x5,0x2,0x0}`.

### 3.4 Startup sequence (libhantek6xx4 `hantek_open_device`)

1. Claim interface 0.
2. Bulk `0C 00` (reset); read FPGA version (first 2 bytes, LE).
3. "Wake" SPI sequence (5 writes, e.g. `08 00 00 77 47 <LVDS_TERM reg> 04 00`).
4. Control `0xA2`/`0x1580`: ID string; parse PCB revision and serial.
5. Control `0xA2`/`0x1600`: calibration table; check flag `0xFBCF`.
6. Bulk `09 00`: hardware revision.

pyhantek's setup instead replays raw ADC init packets, e.g. `08 00 00 77 47 12 04 00`,
`08 00 00 03 00 33 04 00`, `08 00 00 65 00 30 02 00`, `08 00 00 28 f1 0f 02 00`,
`08 00 00 12 38 01 02 00` (sleeps 2/2/2/15 ms), then PLL/config writes. Meaning of many bytes is unknown.

### 3.5 Acquisition flow (pyhantek `read_buffer`, the only end-to-end working capture)

1. `03 00 01 00` start capture
2. `06 00` + 512-byte read (twice)
3. `0D 00` + 512-byte read
4. `0E 00 00 00`
5. `05 00 00 80` (128 packets)
6. ping (control `0xB2`), then bulk read 65536 bytes

### 3.6 Sample format

- 8-bit unsigned samples, **4 channels interleaved byte by byte** (ch1, ch2, ch3, ch4), 16384 samples per channel per 64 KiB read (pyhantek).
- Mid-scale about 127 (SiberK: `volts = (sample - leverPos) * 8 * voltsPerDiv / 255`, leverPos default 127; pyhantek ch1: `v/(255/8) - 3`).
- Vendor DLL (`dsoHTGetData`) returns 16-bit buffers per channel, depth 0x2000 default, 0x4000 max.
- Channel count vs. rate interaction (1 GS/s needs fewer channels / interleaved ADC) is not yet understood.

### 3.7 Ranges (libhantek6xx4)

- 12 V/div settings: 2 mV … 10 V.
- 27 timebase settings: 2 ns … 1 s. (SiberK's app tables: 2 ns … 1000 s, scan/roll mode above index 23.)
- 4 channels, per-channel level, bandwidth limit, AC/DC coupling.

## 4. Open-source projects

| Project | Value | License |
|---|---|---|
| [pvachon/libhantek6xx4](https://github.com/pvachon/libhantek6xx4) | Best protocol reference (C, libusb): startup, SPI map, front-end bits, status. Sample readback incomplete, some values hard-coded. Matches only PID `0x6cde`. 27 commits, last 2019 | BSD-2-Clause |
| [danielkucera/pyhantek](https://github.com/danielkucera/pyhantek) | Only working end-to-end capture (Python, pyusb) plus a scopehal SCPI bridge. Many magic numbers. Labeled "6xx4B", experimental | None stated (reference only, do not copy) |
| [hackhantek/hantek6004](https://github.com/hackhantek/hantek6004) | Early RE work from the Android library. Abandoned Dec 2018 | Unverified |
| [SiberK/HANTEK_DSO_6254BD](https://github.com/SiberK/HANTEK_DSO_6254BD) | C++Builder Windows app wrapping vendor DLLs. No protocol docs, but shows call order, calibration read, tables | None stated |
| [danielkucera/hantek-wine-driver](https://github.com/danielkucera/hantek-wine-driver) | Reportedly runs the official software under Wine (useful for tracing). Not reviewed | Not checked |
| [sigrok Hantek 6254BD](https://sigrok.org/wiki/Hantek_6254BD) | Status "planned"; BOM and lsusb only. No 6254BC page | n/a |

Vendor SDK call order (from SiberK's usage of `HTHardDll`): `dsoHTSearchDevice`, `dsoHTDeviceConnect`,
`dsoInitHard`, `dsoHTADCCHModGain`, `dsoHTReadCalibrationData(dev, CHLevel, 28)`, `dsoHTSetSampleRate`,
`dsoHTSetCHAndTrigger`, `dsoHTSetRamAndTrigerControl`, `dsoHTSetCHPos`, `dsoHTSetVTriggerLevel`,
`dsoHTSetTrigerMode`, `dsoHTStartCollectData` (flags: 0x01 auto, 0x02 roll, 0x04 not collecting),
`dsoHTGetState` (bit 0x02 = data ready), `dsoHTGetData` / `dsoHTGetScanData`.

## 5. Related but NOT applicable

- **OpenHantek / OpenHantek6022, sigrok `hantek-6xxx`, Hantek6022API:** 6022 family only. Different
  protocol (vendor requests `0xE0`–`0xE5`, host-uploaded fx2lafw firmware, VID `04b4/04b5/1d50`, EP `0x86`).
- **mikrocontroller.net "Hantek Protokoll":** DSO5xxxB/DST1xxxB family, framed protocol (`0x53` marker, checksum),
  VID `049f:505a`, EP `0x01/0x82`. Does not mention the 6xxx series.

## 6. Gaps and risks

1. 6254BC USB ID and PCB revision unconfirmed.
2. ADC/PLL init is partly replayed magic bytes; the HMCAD1511 register map (`hmcad1511.h` in libhantek6xx4) was not reviewed.
3. FPGA behavior unknown: horizontal trigger position, pre-trigger depth, memory depth, interleave at 1 GS/s, pulse/video triggers.
4. Calibration data is read by both projects but not fully applied.
5. **Brick risk:** a 6204BD FX2 was bricked by flashing OpenHantek 6022 firmware; a 6104BC was bricked and recovered by writing a 6074BC EEPROM dump. Back up the 24LC64 before any write.
6. Windows driver signing affects only the official software; Linux libusb needs just a udev rule.
7. pyhantek and SiberK have no license; only libhantek6xx4 (BSD-2) can be reused, with attribution.
8. Conflicting reports on the VID the software opens (`04b4` vs `04b5`); verify by capture.

## 7. Sources

- sigrok: [Hantek 6254BD](https://sigrok.org/wiki/Hantek_6254BD), [Info](https://sigrok.org/wiki/Hantek_6254BD/Info)
- [EEVblog: Hantek 6254BD 250MHz 1GSa/s PC/USB DSO](https://eevblog.com/forum/testgear/hantek-6254bd-250mhz-1gsas-pcusb-dso) (pages 1–2 read)
- [libhantek6xx4](https://github.com/pvachon/libhantek6xx4), [pyhantek](https://github.com/danielkucera/pyhantek), [SiberK repo](https://github.com/SiberK/HANTEK_DSO_6254BD)
- [libsigrok hantek-6xxx](https://github.com/sigrokproject/libsigrok/tree/master/src/hardware/hantek-6xxx), [Hantek6022API](https://github.com/Ho-Ro/Hantek6022API)
- [mikrocontroller.net Hantek Protokoll](https://www.mikrocontroller.net/articles/Hantek_Protokoll)

## 8. Findings from decompiling the vendor `HTHardDll.dll` (x86)

Source: `docs/HT6004BX_SDK/Dll/x86/HTHardDll.dll` (98 304 bytes, May 2018), decompiled with Ghidra
headless (`docs/tools/ExportDecomp.java`, run with `analyzeHeadless … -postScript ExportDecomp.java <out.c>`),
plus the SDK headers (`HeadFiles_UTF8/*.h`), the English manual and the QTDSO demo. Function names
below are exported names; internal helpers are Ghidra `FUN_<addr>` names. Items are *from the vendor
code* (strongest evidence so far) but still **not yet replayed on our unit** unless stated.

### 8.1 How the DLL talks to the device

`HTHardDll.dll` imports only KERNEL32 (`CreateFileA`, `DeviceIoControl`, ...) and `HTSoftDll.dll`. It
never touches USB directly: it opens the Windows kernel driver (`Hantek6000BX86.sys`) and uses three
private IOCTLs. Everything else is built in the DLL.

| IOCTL | Meaning | Helper |
|---|---|---|
| `0x222051` | Bulk **OUT** write to EP `0x02` (input buffer = 4 zero bytes, "output" buffer = the command bytes) | `FUN_10001a80(dev, len, buf)` |
| `0x22204E` | Bulk **IN** read from EP `0x86` | `FUN_10001b40(dev, len, buf)` |
| `0x222059` | Vendor **control** transfer; 10-byte header | `FUN_10001180(dev, bRequest, wValue, dir, buf, len)` |

Control header (10 bytes): `[0]=dir (1 read, 0 write) [1]=0x02 [2..3]=0 [4]=bRequest [5]=0 [6..7]=wValue (LE) [8..9]=0`.
Reads/writes longer than 1024 bytes are split in 0x400 chunks with `wValue += offset`.

**Every bulk write is preceded by** `FUN_10001290(dev, 1)`: control OUT `0xB3` with 10 bytes then control IN
`0xB2` (10 bytes). The `0xB3` payload is the "transfer mode" and depends on the mode argument:

| mode | payload (first 4 bytes) |
|---|---|
| 1 (default, used before bulk writes) | `0f 03 03 03` |
| 2 | `0f 0a 0a 0a` |
| 3 | `0f 09 09 09` |
| 4 | `0f 01 01 01` |
| 5 | `0f 02 02 02` |
| 6 | `0f 08 08 08` |

(rest zero). Reading: `FUN_10001360` does control IN `0xB2`; byte 0 non-zero means high speed, so
every bulk **read** size is **0x200 (512)** if byte0 != 0, else **0x40 (64)**. This confirms libhantek6xx4's
`0xB3`/`0xB2` behaviour. Our unit returned `01` (high speed, 512-byte packets).

### 8.2 Control requests used by the DLL (`FUN_10001180` call sites)

| bRequest | wValue | dir | len | Purpose |
|---|---|---|---|---|
| `0xB3` | 0 | W | 10 | Transfer-mode preamble (above) |
| `0xB2` | 0 | R | 10 | Ready / high-speed flag |
| `0xEA` | 0 | W | 10 | `dsoSetUSBBus` — switch the bus path to USB (first thing in init) |
| `0xE5` | 0 | W | 10 | `dsoSetSPIBus` |
| `0xA2` | `0x1580` | R/W | 0x47 | Info string (device name/PCB/serial); `dsoGetHardInfo`/`dsoSetHardInfo` |
| `0xA2` | `0x15E0` | R/W | n*2 | User/driver-info area (`dsoGetDriverVersion`, `dsoReadIIC`/`WriteIIC`) |
| `0xA2` | `0x1600` | R/W | 2n | Calibration table (`dsoHTReadCalibrationData`/`Write…`) |
| `0xA2` | `0x1B00` | R/W | 2n | Second calibration table (`dsoHTRDAmpCali`/`WRAmpCali`); validity flag `0xFACF` |
| `0xA2` | `0x1D80` | R/W | 2n | Third table (DDS/AWG calibration, `dsoGetDDSCali`/`dsoSetDDSCali`) |
| `0xEC` / `0xED` | `0x1700`, `0x1716` | R / W | 0x14, 1 | Unknown small areas (LAN/IP settings? `dsoUSBMode*`) |
| `0xE3` | `0x15F0` | R | 4 | Unknown |
| `0xF4` | `0x1F00` | R | 2 | Unknown (USB module version?) |
| `0xF0` / `0xF1` | `0x1E00` | W / R | 0x40 | **Flash read/write** (`dsoWriteFlash`/`dsoReadFlash`, FPGA bitstream) — **never write** |

Info string slices used by the DLL (offsets into the 71 bytes at `0xA2/0x1580`): device name `[6..14)`,
PCB version `[14..20)`, serial `[34..42)`. On our unit that gives name `6000`, PCB `P1.06`, serial `64C02153`
(slice looks suspect, see §1a); the PCB string is parsed to a number (`1.06` → 106) to pick ADC gain codes.

### 8.3 Bulk commands (EP `0x02`), exactly as built by the DLL

All numbers little-endian. Writes are followed by the `0xB3`/`0xB2` control pair described above.

| Cmd | Bytes | Built by / meaning |
|---|---|---|
| `0C 00` | 2 | Reset; then bulk read: first u16 = **FPGA version** (`FUN_100055c0`, cached) |
| `09 00` | 2 | Hardware version; bulk read, first u32 (`dsoGetHardVersion`) |
| `06 00` | 2 | **State**; bulk read, first u16: bit0 triggered, bit1 data ready, bit3 pack state (`dsoHTGetState`, `GetPackState`) |
| `18 00` | 2 | Init SDRAM (`dsoHTInitSDRam`) |
| `03 00 m 00` | 4 | **Start collect**; `m = nStartControl & 0x7F` (bit0 auto, bit1 roll, bit2 stop after) |
| `0D 00` | 2 | **Buffer status**; reads u16@0, u16@2 (trigger address), byte@4, u16@6, u16@8 |
| `0E 00 lo hi` | 4 | Set readback start address (computed from trigger address, pre-trigger, buffer length) |
| `05 00 lo hi` | 4 | **Read back** `n` bytes: value = `n/2` (u16). Data then arrives on EP `0x86` in 512-byte (or 64) packets |
| `08 00 00 d0 d1 reg cs 00` | 8 | SPI write: 16-bit `d1:d0`, register `reg`, chip select `cs` (1 front-end shift regs, 2 ADF4360-7, 4 HMCAD1511) |
| `0F 00 <u32>` | 6 | Timebase (sample-rate divider), see §8.5 |
| `00/01/02/04 00 lo hi` | 4 | Channel vertical position; id = `0` for CH1 else `1<<(ch-1)` (CH2=1, CH3=2, CH4=4); 10 ms sleep after |
| `07 00 …` | 26 | Trigger level, see §8.6 |
| `10 00 …` | 14 | Horizontal trigger/pre-trigger length (two 40-bit values), see §8.7 |
| `11 00 mode slope coupling 00` | 6 | Trigger mode (`FUN_` in `dsoHTSetTrigerMode`) |
| `12 00 b2 00 b4 b5` | 6 | Trigger source / channel / mode bits, see §8.8 |
| `13 00 …` | 16 | Pulse-width trigger (`dsoHTSetPulseTriger`) |

Trigger mode wire values (`11 00 mode …`): edge `0`, pulse `1`, video `2`, CAN `0x0B`, LIN `0x0A`, UART `0x09`,
SPI `0x0C`, I2C `0x0D`, force `0x80`. `slope`: rise 0 / fall 1. `coupling` is passed through
(0 DC, 1 AC, 2 LF reject, 3 HF reject, 4 noise reject per the manual).
Pulse trigger packet (`13 00`): three u32 fields at bytes 2–5, 6–9, 10–13 (derived from the pulse width by float math,
width unit is 5 ns / 10 ns per the manual), byte 14 = `condition << 1`, byte 15 = 0. Constants for the float math not decoded yet.

### 8.4 Initialization sequence (`dsoInitHard` → `FUN_100081b0` + `FUN_10001d30`)

1. Control OUT `0xEA` (`dsoSetUSBBus`), 10 bytes (content not significant).
2. Bulk `0C 00` reset, bulk read → FPGA version (u16).
3. Control IN `0xA2/0x1580` → info string (0x47 bytes).
4. Read DDS calibration (`0xA2/0x1D80`), amplitude calibration, ADC calibration tables (`0xA2/0x1B00`, `0x1600`);
   if a table fails its validity flag (`0xFBCF` for `0x1600`, `0xFACF` for `0x1B00`) the DLL substitutes defaults
   (value `0x400`; some default blocks use `0x40DA, 0xEACF, 0x4478, 0xE741, 0x452E, 0xE614`).
5. ADC/PLL bring-up over SPI (confirms pyhantek's bytes):
   - `08 00 00 77 47 12 04 00` (HMCAD1511 reg `0x12`, value `0x4777`), sleep 2 ms
   - `08 00 00 03 00 33 04 00` (HMCAD1511 reg `0x33`, value `0x0003`)
   - then, with `cs = 2` (ADF4360-7), each followed by the sleep before it: `08 00 00 65 00 30 02 00` (sleep 2),
     `08 00 00 28 f1 0f 02 00` (sleep 2), `08 00 00 12 38 01 02 00` (sleep 15)
6. Application then calls `dsoHTADCCHModGain`, calibration reads, `dsoHTSetSampleRate`, `dsoHTSetCHAndTrigger`,
   `dsoHTSetRamAndTrigerControl`, `dsoHTSetCHPos` per channel, `dsoHTSetVTriggerLevel`, `dsoHTSetTrigerMode`
   (the QTDSO demo `Hard::Init` does exactly this order).

### 8.5 Timebase divider (`0F 00 <u32>`, `FUN_10002330`)

Payload is `divisor − 1` (u32 LE) with `divisor` chosen by timebase index (0 = 2 ns … 35 = 1000 s):

| idx | 0–9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35–36 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| divisor | 1 | 2 | 5 | 10 | 25 | 50 | 100 | 250 | 500 | 1000 | 2500 | 5000 | 10000 | 25000 | 50000 | 100000 | 250000 | 500000 | 1e6 | 2.5e6 | 5e6 | 1e7 | 2.5e7 | 5e7 | 1e8 | 2.5e8 | 5e8 |

This matches pyhantek (`raw = 250e6/rate/2 − 1`, e.g. 200 µs/div → 99) and the manual's "sample rate = 250 / timebase"
(250 points per grid). Indices 0–9 (2 ns … 2 µs) all send `0`; the actual 1 GS/s / 500 MS/s interleave for the fastest
bases is done by the ADC channel-mode registers (§8.9), plus software interpolation (DefMacro: `MAX_INSERT_TIMEDIV = 6`).
Sample-rate by channel mode (manual Table 1): idx 0–6 single channel 1 GS/s, dual 500 MS/s, quad 250 MS/s;
idx 7 dual 500 MS/s, quad 250 MS/s; idx ≥ 8 250 MS/s / divisor.

### 8.6 Trigger level (`07 00 …`, 26 bytes, `FUN_10001dc0`)

`mid = f(nPos)` (float math, `nPos` 0–255), `hi = min(mid + sens, 0xE4)`, `lo = max(mid − sens, 0)` with `sens` = `nSensitivity`
(demo passes 4). Layout: bytes 0–1 `07 00`; then four groups `hi hi lo lo` (bytes 2–17); then eight bytes of `mid` (bytes 18–25).
The `mid(nPos)` mapping is not decoded (pyhantek uses `((V+1)/4)*200 + 28.5`; "28.5" matches the `32.5`/`28` offsets seen in the DLL, to verify).

### 8.7 Horizontal trigger / pre-trigger length (`10 00 …`, 14 bytes, `FUN_10002060`)

Two 40-bit little-endian values (bytes 2–6 and 8–12 each with a high byte; bytes 7 and 13 hold bits 40–47 as written),
computed from buffer length (`FUN_10001450`: **0x10000 (64K)** normally, **0x8000 (32K)** for buffer-index 3–7),
the horizontal trigger position (%), channel mode and timebase. Details are float-heavy and still to decode; both
open-source projects hard-code the two values.

### 8.8 Trigger source (`12 00 b2 00 b4 b5`, `FUN_100095d0`)

Inputs: timebase idx `t`, channel mask `m` (CH1 = bit0), trigger source `s`, peak-detect flag `p`; `n` = number of enabled channels.
- `b2 = ((p & 1) << 6) | ((m & 0xF) << 2) | x`
- `b4 = y & 3`
- `b5 = ((~(m >> s) & 1) << 2) | z`
- `x, y, z` depend on `n` and `t`: `n=1`: `x = 0 if t<7; 1 if t==7; 2 if t==8; 3 if t>8`, `y = 3 − x`;
  `n=2`: `x = 0 if t<8; 1 if t==8; 2 if t>8`, `y = 2 − x`; `n=3,4`: `x = (t>8)`, `y = 1 − x`.
  `z` derives from the channel mode (1: 0; 2: `(n==1 ? (m is CH1 or CH3 ? 0 : 1) : (m>>s)&1 == 1)`; 3/4: `s`).
(Reading of the decompiler output; double-check against a trace.)

### 8.9 Front-end relays and ADC channel mode

**Shift register** (`dsoHTSetCHAndTrigger` → `FUN_100030b0`): message `08 00 c0 c1 c2 c3 01 00` (cs = 1). Per channel byte:
bit7 bandwidth-limit, bit6 range > 1 V (vdiv idx > 8), bit5 range ≤ 1 V, bit4 range > 100 mV (idx > 5), bit3 range ≤ 100 mV,
bit2 DC coupling (coupling == 0), bit1 always 1, bit0 0. The DLL waits on a busy flag (`FUN_10001c90`), sends the
message, sleeps **4 ms**, then sends it **again with each byte `& 0x86`** (relay-drive bits cleared, bandwidth bit and
coupling/base bits kept), then sleeps **50 ms**. So the relays are pulsed (latching relays), not "latch 0 then 1" as
libhantek6xx4 describes — an earlier note in §3.3 should be read with this correction.

**HMCAD1511 configuration** (`FUN_10002830`, called from `SetCHAndTrigger` and `SetSampleRate`): per-channel gain code by
vdiv idx: `0→0xD, 1→0xA, 2→0x7, 3/6/9→0x5, 4/7/10→0x2, else 0` (matches libhantek6xx4's table, with 12 vdivs 2 mV…10 V),
packed into a 16-bit word (one nibble per channel). Then SPI writes with `cs = 4`, `reg` / value depends on the number of
enabled channels and the timebase:
- reg `0x3A` and `0x3B`: input-select words (`0x1010 / 0x0808 / 0x0404 / 0x0202` etc. one-hot per enabled channel;
  quad mode `0x810` and `0x204`), 3 ms sleeps between writes
- reg `0x0F`: `0x0000`
- reg `0x31`: channel mode word (`0x0001` single, `0x0102` dual, `0x0204` quad)
- reg `0x2A` (quad mode) or `0x2B` (single/dual): the packed gain nibbles
`dsoHTADCCHModGain(dev, nCHMod)`: `08 00 00 g 00 55 04 00` (reg `0x55`, cs 4) where `g` depends on the PCB revision parsed from the
info string: rev 105 → `{1ch:0x00, 2ch:0x0A, 4ch:0x37}`; rev 106 → computed as `clamp(k·333.33 + 32.5, 0, 63)` with
`k = 1.10, 1.08, 0.95` which all clamp to **63 (0x3F)**; other revisions → `{0x19, 0x30, 0x3F}`. **Our unit is PCB 1.06 → `0x3F` for all channel modes**
(matches pyhantek's `08 00 00 3f 00 55 04 00`).

### 8.10 Capture and data read (`dsoHTGetData` → `FUN_10004100`, `FUN_100041f0`, `FUN_10004240`)

1. Start: `03 00 m 00`; poll `06 00` until `state & 2`.
2. `0D 00` → buffer status (trigger address in u16@2, etc.).
3. `0E 00 lo hi` with the computed read start address (address is wrapped by the buffer length 64K/32K, rounded to a multiple of 8, adjusted by channel mode).
4. `05 00 lo hi` with `n/2`, then bulk-read `n` bytes in 512-byte (or 64-byte) packets.
5. De-interleave: samples are **one byte per channel**, interleaved in groups of *k* bytes where *k* = number of enabled channels
   (1, 2 or 4; 3 enabled channels use the 4-channel layout). The DLL widens each byte to `WORD`.
Zero level: the demo's `SourceToDisplay` computes `sample − (255 − leverPos)` with `leverPos` (CH1 default 192, CH2 160, CH3 96, CH4 64) = channel position (0–255).
Buffer lengths: `BUF_4K_LEN` (the demo reads 4096 samples). Scan (`dsoHTGetScanData`, timebase ≥ 25) and roll
(`dsoHTGetRollData`, ≥ 27) modes use the same primitives with incremental reads.

### 8.11 AWG / DDS (`dds*` exports, not yet decoded)

The 6254BC has a DAC902E-based arbitrary generator. The SDK API is: `ddsSetOnOff`, `ddsSetFrequency`, `ddsDownload` (wave RAM, 12-bit points),
`ddsSetCmd` (0 continuous / 4 burst / 1 single), `ddsSDKSetFre/Amp/Offset/BurstNum/WaveType (0 sin, 1 ramp, 2 square, 4 DC, 8 noise)/WavePhase/WaveDuty`,
`ddsSetSweep`, `ddsSetFAOC`, `ddsSetSyncOut`, `ddsSetAMFMFreq`. Functions are at `0x100086a0`–`0x10009120` in the decompile for later work.

### 8.12 Corrections to earlier sections

- §3.3: the relay update sends the register image twice (full, then `& 0x86`) with 4 ms / 50 ms sleeps; it is not a 0/1 latch.
- §3.1: control `0xEA` is `dsoSetUSBBus` (write), `0xE5` is `dsoSetSPIBus`; `0xF0/0xF1 0x1E00` are the flash write/read requests (FPGA bitstream) — avoid.
- §3.2: `0x18 00` (init SDRAM) and `0x13 00` (pulse trigger) are real; `0x1E…` in pyhantek is not seen in the vendor DLL.
- §1a info string: serial slice is `[34..42)` and name `[6..14)`, PCB `[14..20)` per the DLL.

### 8.13 Still unknown / to decode next

1. `mid(nPos)` trigger level mapping (§8.6) and the exact `10 00` horizontal-position fields (§8.7): need the float constants from `.rdata` at `0x10012100`–`0x10012360`.
2. `dsoHTSetCHPos` mapping from `(vdiv, pos, channel mode)` to the 16-bit offset, which uses the calibration table (`0xA2/0x1600`, layout 9 points × 4 channels × …).
3. Buffer-address arithmetic in `dsoHTGetData` (§8.10, step 3) and sample interpolation (`HTSoftDll.dll`).
4. Pulse/video/serial triggers (`13 00`, `dsoHTSetVideoTriger`).
5. Calibration table layout and write path (`dsoHTSetAmpCalibrate`, `dsoHTWriteCalibrationData`), and AWG (`dds*`).
6. What `0xEC/0xED/0xE3/0xF4` read/write.
7. Replay on our unit: bulk `0C 00`, `09 00`, `06 00`, then the init sequence above and one capture.

## 9. Replay on our 6254BC unit (2026-11-20) — VERIFIED

Throwaway probe: `docs/tools/replay_probe/` (Rust + rusb; `cargo run --release -- <stage>`). It implements the
vendor-DLL transport from §8.1 with direct libusb calls (vendor control `0xB3`/`0xB2` before every bulk write,
512-byte bulk reads). No EEPROM/flash writes were made. Stages: 1 = reset/versions/state, 2 = + init, 3 = + configure + capture.

**Verified working:**
- Bulk `0C 00` reset → first u16 of the reply = FPGA version **`0xD001`** (reply is a 512-byte packet repeating `01 d0`).
- Bulk `09 00` hardware version → all zero on this unit.
- Bulk `06 00` state → first u16: `0x0000` before init; after the ADC/PLL init `0x8402`; after start-collect `0xC100`; when data is ready `0xC402`
  (bit1 = data ready as documented; other bits not decoded). The reply is a full 512-byte packet; the state is in the first bytes.
- SPI init (§8.4: HMCAD1511 `0x12`/`0x33`, ADF4360 `0x30`/`0x0F`/`0x01`, ADC gain reg `0x55` = `0x3F`) accepted, no errors.
- Full configuration with the decoded commands (timebase idx 15 = `0F 00 63 00 00 00`, ADC 4-channel mode regs `3A/3B/0F/31/2A`,
  relays `08 00 36 36 36 36 01 00` then `… 06 06 06 06 01 00`, trigger source `12 00 3d 00 00 00`, channel positions, trigger level,
  trigger mode `11 00 00 00 00 00`, horizontal position `10 00 …`) followed by `03 00 01 00` start.
- Capture: data ready after < 200 ms; `0D 00` status reply `f8 70 20 91 07 12 55 55 f8 70 20 91`; then `0E 00 00 00` + `05 00 00 80`
  returned **65 536 bytes** (128 × 512) in one bulk read sequence. 4 channels interleaved byte-by-byte; with nothing connected each
  channel is a constant level set by the channel position: CH1 101, CH2 124, CH3 109, CH4 128 (positions `0x6201`, `0x7064`, `0x6980`, `0x72D9`
  taken from the pyhantek capture — the formula for these is still undecoded).

**Observed differences vs. the vendor DLL:**
- Control `0xEA` OUT with 10 zero bytes **stalls (`Pipe`)** on our unit. Everything works without it, so it is not required (the DLL passes an
  uninitialised 10-byte stack buffer; maybe it needs particular content or is an IN request). Treat as optional.
- The device already answered state/versions before init, so the reset `0C 00` alone is enough to talk to it.

**Not yet verified:** a real signal capture (calibrator), the trigger level/position math, other timebases and channel modes, trigger types,
roll/scan modes, and the AWG.

### 9.1 Real-signal capture and position sweep (2026-11-20, test signal on CH1)

Setup: probe-compensation output on CH1, stage 3 settings (4-channel mode, 1 V/div relays `0x36`, timebase idx 15, auto trigger).
- CH1 captured a clean square wave: two plateaus at counts ≈ 103 and ≈ 149 (~8 000 samples each), edges spaced exactly **625 samples**,
  full period **1250 samples**. With a 1 kHz signal this proves **1.25 MS/s** for timebase idx 15 in 4-channel mode, i.e. the divider table of §8.5
  (`0F 00 63 00 00 00` ⇒ 250 MS/s / 200 = 1.25 MS/s... as in the vendor manual: 250 points per grid).
- One bulk readback of `05 00 00 80` = 65 536 bytes = 16 384 samples per channel = 13.1 ms at that rate (about 6.5 grid divisions of 2 ms).
- Amplitude: 46 counts peak-to-peak. With the vendor/pyhantek scale of 32 counts per division at 1 V/div (255 counts = 8 div) that is about 1.4 V p-p;
  the real calibrator amplitude is still to be confirmed.
- CH2–CH4 (nothing connected) stay flat; tiny noise of ±1 count on CH2 (crosstalk from CH1).

**Channel position word** (`0x00/0x01/0x02/0x04 00 lo hi`), measured on unconnected CH4 (1 V/div, 4-channel mode):

| word | level (counts) | | word | level |
|---|---|---|---|---|
| `0x2000` | 0 (clipped) | | `0x6800` | 112 |
| `0x2800` | 8 | | `0x7000` | 125 |
| `0x3000` | 21 | | `0x7800` | 138 |
| `0x4000` | 47 | | `0x8000` | 151.5 |
| `0x5000` | 73 | | `0x9000` | 178 |
| `0x6000` | 99 | | `0xA000` | 204 |

It is **linear: ≈ 13 counts per 0x800, i.e. `word ≈ 8980 + 157.5 × level`** (mid-scale 128 ⇒ ≈ `0x71D4`; CH4's pyhantek value `0x72D9` gave 128–130).
The vendor DLL derives the word from the calibration table (`0xA2/0x1600`, 576 u16 = 4 channels × 12 vdivs × 12 entries, indexed by channel,
vdiv group and channel mode, using adjacent pairs `a,b` per entry) and `nPos`; the slope/offset above are valid for this range only
and may change with V/div, so the calibrated formula is still to decode.

**Trigger level** (decoded from `FUN_10001dc0`): `mid = trunc(nPos × 200/256 + 28.5)` (nPos 0–255), `hi = min(mid + sens, 0xE4)`,
`lo = max(mid − sens, 0)`, sens = `nSensitivity` (demo: 4). Packet `07 00` + 4×`hi hi lo lo` + 8×`mid`. This replaces the unexplained pyhantek formula.

### 9.2 Trigger, buffer status and horizontal position — VERIFIED (2026-11-20)

Test: 1 kHz square wave (≈103/149 counts) on CH1, 4-channel mode, 1 V/div, timebase idx 15 (800 ns per sample), rising edge on CH1,
buffer length B = 4096 samples, normal (non-auto) trigger `03 00 00 00`.

**Trigger level.** `07 00` packet carries ADC count units: with `mid` (§9.1 formula) the capture completes only when `mid` lies between the
signal plateaus (levels 103–141 triggered; 153+ and ≤ 91 did not). So `mid ≈ ADC counts` (0…`0xE4`), not an inverted display scale.

**Status word (`06 00`, first u16).** `0xC200` armed/waiting; `0xC100` just started; **`0xC403` = triggered (bit0) + data ready (bit1)**;
`0xC402` = data ready without trigger (auto mode / forced). In normal mode with no trigger the state never gets bit1.

**Buffer status (`0D 00`, 12 useful bytes).** After a triggered capture: `u16@0` = **byte address of the trigger edge** in the 64 KiB capture
RAM (measured: within ~20 bytes of the edge found in a full-buffer readback), `u16@2` = end address = `u16@0 + 0x2028` for B = 4096 at this
setting (8232 bytes = 2058 frames post-trigger), byte@4 `06`/`07`, byte@5 `0x13`, `u16@6` = `0x5555`, `u16@8` = copy of `u16@0`. If not triggered,
`u16@0/@2` are stale and unrelated.

**Horizontal trigger position (`10 00`, 14 bytes)** — decoded from `FUN_10002060` and verified at 10 %, 50 % and 90 %:
- inputs: buffer length `B` (samples, ≤ 64 K; demo = 4096), horizontal position `h` in %, sample interval `f` in ns (= 1/sample-rate:
  1/2/4 ns for timebase idx ≤ 7 by 1/2/4 channels, then 4, 8, 20, 40, 80, 200, 400, 800 … ns for idx 8…), FPGA version → latency `lat`
  (**230** for our FPGA `0xD001`; **246** only for FPGA versions `0xA000`/`0xA00A`/`0xA00B`).
- `pre = trunc(h · B / 100)`
- `first  = trunc((lat + (pre + 100) · f) / 8)` and `second = trunc(((B − pre + 8) · f) / 8)`; if `f < 200` each result is rounded **up** when it has a fraction.
- packet: `10 00`, `first` as 6-byte LE, `second` as 6-byte LE (units of 8 ns = 125 MHz clock).
- check: B = 4096, h = 50, f = 800 → `first = 214828`, `second = 205600`, identical to the constants hard-coded in pyhantek/libhantek6xx4.
- measured (rising edge position in the 4096-sample window): h = 10 % → frame 403–404 (ideal 409); h = 50 % → 2042–2043 (2048); h = 90 % → 3680–3681 (3686).
  The trigger lands about 5 frames (≈ 4 µs at this rate) earlier than ideal in every case, jitter ±1–2 frames.

**Trigger-aligned readback** (verified): after the status word shows ready,
1. `0D 00` → read status, `end = u16@2`;
2. `start = (end − 4·B·?) mod 0x10000`, aligned down to a multiple of 8 — with 4 channels `start = (end − 4·B) & 0xFFFF & ~7` (= `end − 16384` for B = 4096);
3. `0E 00 lo hi` with `start`;
4. `05 00 lo hi` with `value = (4·B)/2` (= `0x2000` for 4096 samples × 4 channels) → read `4·B` bytes (512-byte packets).
The 4096-sample window then has the trigger edge at `h` % of the window. For fewer channels the byte count per sample frame is the channel count
(1, 2 or 4); the exact end-address arithmetic for 1/2-channel modes still has to be checked against the DLL (`dsoHTGetData`).
Reading the whole RAM with `0E 00 00 00` + `05 00 00 80` (64 KiB, as pyhantek does) also works but the trigger edge sits at an arbitrary place (`u16@0`).

## 10. Full configuration model — implemented and VERIFIED (2026-11-20)

Reference implementation of everything below: `docs/tools/config_probe/src/main.rs` (`cargo run --release -- modes|pos|fast`).
It follows the vendor DLL order of `Hard::Init`: ADC gain reg → sample rate (ADC regs, timebase `0F`, horizontal `10`) → relays + ADC regs
→ trigger source `12` → channel positions → trigger level `07` → trigger mode `11` → start `03`.

### 10.1 Data layout (VERIFIED)

The readback stream has **one byte per enabled channel per sample frame**: `n = 1` → 1 byte/frame, `n = 2` → 2 bytes/frame (enabled channels in
ascending order), `n = 3` or `4` → 4 bytes/frame (CH1…CH4). Measured: a 1 kHz square wave on CH1 has period **1250 frames in 1-, 2- and 4-channel
modes at timebase idx 15**, and `n` bytes per frame. The ADC "mode" `k` (§10.2) only affects the ADC clocking, not the layout.
**RAM contains stale data:** only the first `B × n(or 4)` bytes of a fresh capture are valid (the window of B = 4096 samples); the rest of the 64 KiB is
left over from older captures, and the first 2–3 frames of an auto capture read from address 0 may also be stale. Always read the trigger-aligned window (§9.2).

### 10.2 ADC channel mode and gains (`FUN_10002830`, VERIFIED on 1/2/4 channels and t = 3, 6, 7, 8, 15)

Let `n` = enabled channels, `t` = timebase idx (0 = 2 ns), `g[i]` = gain code of channel `i` (CH1 = 0) from its V/div idx:
`0→0xD, 1→0xA, 2→0x7, 3/6/9→0x5, 4/7/10→0x2, 5/8/11→0x0`.
Mode `k` (also used for the sample interval): `n ≥ 3` → 4; `n = 2` → 2 if `t ≤ 7` else 4; `n = 1` → 1 if `t < 7`, 2 if `t = 7`, 4 if `t > 7`.
(1 GS/s single, 500 MS/s dual, 250 MS/s quad.) Registers written with `08 00 00 d0 d1 reg 04 00` (HMCAD1511, 16-bit `d1:d0`), 3 ms apart, in this order:

| reg | value |
|---|---|
| `0x3A` | quad: `0x0810` · n=1,t<7: `0x1010/0x0808/0x0404/0x0202` for CH1/2/3/4 · n=1,t=7: CH1 `0x1010`, CH2 `0x0404`, CH3 `0x0404`, CH4 `0x1010` · n=2,t<8: pair CH1+2 `0x1010`, 1+3 `0x1010`, 2+3 `0x0808`, 1+4 `0x1010`, 2+4 `0x0808`, 3+4 `0x0404` |
| `0x3B` | quad: `0x0204` · n=1,t<7: same as 3A · n=1,t=7: CH1 `0x0808`, CH2 `0x0808`, CH3 `0x0202`, CH4 `0x0202` · n=2,t<8: pair 1+2 `0x0808`, 1+3 `0x0404`, 2+3 `0x0404`, 1+4 `0x0202`, 2+4 `0x0202`, 3+4 `0x0202` |
| `0x0F` | `0x0000` |
| `0x31` | k=4: `0x0204`; k=2: `0x0102`; k=1: `0x0001` |
| `0x2A` (k = 4) | gain word `g4<<12 \| g3<<8 \| g2<<4 \| g1` (`g1` = CH1) |
| `0x2B` (k = 1, 2) | build `word` (n=1: `g·0x1111` of the enabled channel; n=2: low byte `g(first)·0x11`, high byte `g(last)·0x11`), then `d0 = ((hi ^ lo) & 0xF) ^ hi`, `d1 = lo & 0xF` |

Plus once per configuration: `08 00 00 3F 00 55 04 00` (reg `0x55`, PCB 1.06 → `0x3F`).

### 10.3 Relays (VERIFIED), trigger source (VERIFIED), timebase (VERIFIED), horizontal (VERIFIED)

- Relays: `08 00 c1 c2 c3 c4 01 00`, sleep 4 ms, same with each byte `& 0x86`, sleep 50 ms; per-channel byte as in §8.9 (bit7 BW limit, bit6 V/div > 1 V (idx > 8), bit5 ≤ 1 V, bit4 > 100 mV (idx > 5), bit3 ≤ 100 mV, bit2 DC, bit1 = 1).
- Trigger source `12 00 b2 00 b4 b5` as §8.8 (with `n`, `t`, `mask`, source `s`, peak `p`; `uv6 = mask >> s`):
  `b2 = p<<6 | mask<<2 | x`, `b4 = y & 3`, `b5 = (~uv6 & 1)<<2 | z`; `n=1`: `x = 0 (t<7), 1 (t=7), 2 (t=8), 3 (t>8)`, `y = 3−x`;
  `n=2`: `x = 0 (t<8), 1 (t=8), 2 (t>8)`, `y = 2−x`; `n≥3`: `x = (t>8)`, `y = 1−x`;
  `z = 0` (k=1) · k=2: `n=1 ? (mask is CH1 or CH3 → 0 else 1) : (uv6 == 1)` · k=4: `s`.
- Timebase `0F 00 <divisor−1 as u32>` per §8.5; sample interval `f` (ns): `k` ns for `t ≤ 7`, else 4, 8, 20, 40, 80, 200, 400, 800, 2000 … for t = 8, 9, 10, …
- Horizontal `10 00 …` per §9.2 (B = 4096, 50 %).

### 10.4 Channel position from the calibration table (VERIFIED for V/div ≥ 100 mV)

Calibration table = 576 u16 + flag (`0xA2/0x1600`, 1154 bytes, flag `0xFBCF`). For channel `ch` (CH1 = 0), V/div idx `v`, mode `k`:
- `row = 5` for `v ≤ 5`, `8` for `v = 6..8`, `11` for `v = 9..11`;  `m = 4` (k=4), `2` (k=2), `0` (k=1);  `idx = m + 12·(12·ch + row)`;  `a = W[idx]`, `b = W[idx+1]`.
- if `v ∈ {5, 8, 11}`: `lo = a`, `hi = b`. Otherwise `avg = (a + b + 1) >> 1`, `mul = 0.02 (v=0), 0.05 (v=1), 0.1 (v=2), 0.2 (others)`,
  `d = floor((b − avg)·mul + 0.5)`, `lo = avg − d`, `hi = avg + d`.
- `word = trunc((hi − lo)·nPos/255 + lo)` for `nPos` 0–255; sent as `<id> 00 lo hi`, id = `0` for CH1, `1 << (ch−1)` for CH2–CH4 (so CH2 = `01`, CH3 = `02`, CH4 = `04`).

Measured baseline level (counts, open input, DC) for `nPos` = 0 / 128 / 255, CH3/CH4, k = 4 (1- and 2-channel modes give the same within ~1 count):

| V/div idx | 5 (100 mV) | 6 (200 mV) | 8 (1 V) | 9 (2 V) | 11 (10 V) | 7 / 10 (500 mV, 5 V) | 4 (50 mV) |
|---|---|---|---|---|---|---|---|
| nPos 0 | 32.9 | 33.8 | 31.5 | 33.6 | 31.5 | 90.0 | 109.8 / 94.2 |
| nPos 128 | 132.5 | 130.5 | **128.0** | 130.3 | **128.0** | 129.0 | 135.5 |
| nPos 255 | 228.6 | 226.9 | 225.0 | 226.6 | 225.0 | 167.0 | 173.7 |

So `nPos` 0…255 maps linearly to **≈ 32…225 counts** (centre 128) at the "full" ranges (1 V, 10 V) and to ≈ 90…167 for the 500 mV / 5 V groups (the vendor halves their span with `mul = 0.2`).
For idx 0–4 (2 mV … 50 mV) the open-input baseline is far from 128 (e.g. idx 0 at nPos 128 → 204/120); these ranges amplify the floating input's offset and need a **shorted input** to judge — to re-measure.

### 10.5 What is verified vs. open after this round

Verified on the 6254BC: transport, init, ADC modes for n = 1/2/4 at t = 3, 6, 7, 8, 15 (valid, flat baseline 128), relay bytes for all 12 V/div (results above), trigger source for the tested combinations,
trigger level/position, data layout, calibration-based position for V/div idx 5–11.
Open: true sample rate for the 1 GS/s / 500 MS/s modes (needs a fast signal), amplitude calibration in volts per count (`dsoHTSetAmpCalibrate`, `0xA2/0x1B00` table), V/div idx 0–4 with shorted input, AC coupling and bandwidth-limit relays,
pulse/video triggers, roll/scan modes, AWG, the stale-data behaviour in auto mode, and the control requests `0xEC/0xED/0xE3/0xF4`.

### 10.6 Shorted-input measurements (CH1 shorted, 2026-11-20) — VERIFIED

`cargo run --release -- short|bw` in `docs/tools/config_probe/` (env `BW=1` sets the bandwidth-limit relay bit, `AC=1` clears the DC bit).

**Position formula (§10.4) is correct for all 12 V/div settings**, in both 4-channel and 1-channel modes. Mean level (counts) at `nPos` = 0 / 128 / 255:

| V/div idx | nPos 0 | nPos 128 | nPos 255 | noise sd (counts) |
|---|---|---|---|---|
| 0 (2 mV) | 38–39 | 136 | 230–233 | 19–24 |
| 1 (5 mV) | 33.8 | 131–133 | 228 | 8.6 |
| 2 (10 mV) | 32.7–33.3 | 129–130 | 226 | 4.1 |
| 3 (20 mV) | 31.5–32.0 | 128.9 | 224.8 | 2.1 |
| 4 (50 mV) | **89.3** | 128.0 | **166.6** | 0.9 |
| 5 (100 mV) | 31.1 | 128.1 | 224.4 | 0.5 |
| 6 (200 mV) | 31.5 | 128.3 | 224.4 | 0.5 |
| 7 (500 mV) | **89.0** | 128.0 | **166.1** | ≤ 0.05 |
| 8 (1 V) | 31.0 | 128.0 | 224.0 | 0 |
| 9 (2 V) | 31.7 | 128.2 | 224.5 | 0.4 |
| 10 (5 V) | **89.0** | 128.0 | **166.1** | 0 |
| 11 (10 V) | 31.0 | 128.0 | 224.0 | 0 |

So a shorted input reads 128 ± noise at `nPos` 128 for every range; the 2 mV and 5 mV rows are offset by a few counts (residual amplifier offset, the
calibration table has no separate row for them). The spans are ≈ 31…224 counts, except idx 4, 7, 10 (the `mul = 0.2` group) which span ≈ 89…166.
Idx 0–3 are dominated by input noise (sd 20 counts at 2 mV).

**Bandwidth-limit bit (relay byte bit 7) works:** on the shorted input the noise standard deviation drops from 21.4 → 9.8 (2 mV), 8.5 → 4.2 (5 mV), 2.1 → 1.0 (20 mV).

**AC coupling:** clearing relay-byte bit 2 (DC → AC) is accepted and the shorted-input result is unchanged (as expected); the DC/AC difference itself needs a signal with
a DC offset to confirm.

Remaining open (updated in 10.7-10.9: volts scale, AC coupling and the sample rate are now verified): pulse/video triggers, scan/roll, AWG, `0xEC/0xED/0xE3/0xF4`.

### 10.7 Amplitude table (`0xA2/0x1B00`, per-unit gain trim) — VERIFIED

Control read `0xA2`, wValue `0x1B00`, 578 bytes: 288 little-endian words (4 channels x 12 rows x 6 slots) followed by the
flag word `0xFACF`. If the flag is wrong the vendor code uses no correction. Values are gain factors in 1/1024 units
(`0x400` = 1.0). Rows are the V/div index remapped to 5 (idx 0-5), 7 (idx 6-8) or 9 (idx 9-11); only those rows (plus a few
words of row 8) hold data. Slot 0, 1, 2 = ADC channel mode k = 1, 2, 4 (the other three slots stay `0x400`). On the 6254BC
the trims are within a few percent, differ per channel (500 mV row, slot 0: CH1 `0x455`, CH2 `0x440`, CH3 `0x445`,
CH4 `0x454`) and are near 1.0 in the four channel mode.

The vendor applies the table with command `0x1E` (`dsoHTSetAmpCalibrate`), 26 bytes: `1e 00`, four trims as u16 LE
(bytes 2..9), then for each trim other than `0x400` a 4-byte LE offset word `trunc(p' * (1024 - t) + 0.5)`
(`+0x2000000` if negative) at bytes `10 + 4i`, with `p' = trunc((position - 128) * 0.78125 + 128.5)`. For one channel
mode the single enabled channel supplies the trim and position for all four words; for two channel mode the first
enabled channel's trim fills all four and positions are `[first, first, second, second]`; for four channel mode each
channel uses its own. **Effect, verified on hardware:** the FPGA multiplies the samples by `t/1024` around the zero
level (sending `0x440` for CH2 at 500 mV/div raised the signal above zero by 1.06x while the zero stayed at code 128). It
persists across captures. The library sends it at the end of `configure`. Note that one enabled channel at a timebase
index above 7 runs in ADC channel mode 4, so slot 2 applies there, not slot 0.

### 10.8 Counts to volts

The vendor SDK demos assume `volts = (code - (255 - position)) * 8 * V/div / 255`, i.e. 32 counts per division and a
zero code of `255 - position`. **That does not hold on the 6254BC.** Measured instead, with a 0.994 V DC source
(multimeter) on CH2 and CH1 shorted as the zero reference:

| ADC channel mode k | counts per division | zero code at position 0 / 255 | same, ranges idx 4, 7, 10 |
|---|---|---|---|
| 4 (3-4 channels, slower timebases) | 24.15 | 31 / 224 | 89 / 166 |
| 2 | about 21.6 | about 39 / 216 | 92 / 162 |
| 1 (one channel, timebase idx < 7) | about 21.1 (with the factory trim) | about 43 / 212 | 99 / 156 |

Position 128 is code 128 in every mode, and the zero span is 8 divisions in each mode (the position control always covers
+-4 divisions). Counts per division were checked from 200 mV to 1 V per division at several positions in mode 4 (24.1 to
24.2), and at 500 mV and 1 V in the other modes. The 1 kHz calibrator then reads about 1.9 V, not the nominal 2 V.
Zero levels vary by a few counts between ranges; the table gives typical values, so volts can be off by 2-3 counts
(up to about 5 % of a division-scale reading) at extreme positions in the fast modes. Ranges below 200 mV/div and above
1 V/div have not been checked against a reference. The scale comes from one unit.

### 10.9 AC coupling and sample clock — VERIFIED (CH2 on a 0.994 V DC source, CH4 on the 1 kHz calibrator)

- **AC coupling** (relay byte with the DC bit cleared, set by `Coupling::Ac`): CH2 reads 1.000 V in DC and about
  -0.005 V in AC (the DC level is blocked). CH4's calibrator square wave reads 0..1.9 V in DC and -0.95..+0.95 V in AC
  (mean 0.009 V). Plateau separation on CH4 is 1.908 V and matches CH2's 1.907 V, so the channels agree to 0.1 %;
  the calibrator's real output is about 1.91 V, not the nominal 2 V.
- **Sample clock**: the mean calibrator period measured on the time axis is 1000.000 us (+-0.001 %) at 200 us to 50 ms per
  division (up to 818 periods averaged), and the 500 us half period holds at 50 us per division (idx 13). So
  `sample_interval_ns` (= ns per division / 250 for idx >= 8) is exact from idx 13 to 22.
- **Fast timebases (idx 0..12)**: the calibrator is too slow for a period measurement, but its edge has a finite
  rise time. The 10-90 % rise measured about 240-256 ns at every timebase from 20 us/div (80 ns interval) to 2 ns/div
  (1 ns interval), e.g. 4 samples at 80 ns, 31 at 8 ns, 63 at 4 ns and about 250 at 1 ns. The intervals are therefore
  consistent with the verified clock, including the 1 GS/s one channel mode and the 500 MS/s two channel mode at
  500 ns/div. Sample amplitudes in modes 1 and 2 are lower than in mode 4 (see 10.8).

