# Hantek 6xx4 USB Protocol Specification

This document specifies the USB protocol of the Hantek 6xx4-series oscilloscopes as implemented by
[`hantek6xx4-lib`](../README.md). It was obtained by reverse engineering, since the vendor does not publish it
(see [research notes](research/README.md) for how). Every statement carries one of three evidence levels:

| Label | Meaning |
|---|---|
| **Verified** | Exercised on a real device and the result checked (against a known signal, a multimeter or the data itself) |
| **Vendor** | Taken from the decompiled vendor library; accepted by the device but its effect was not independently checked |
| **Unverified** | Third-party claim or inference; not used by the library |

**Reference unit.** All verification was done on one Hantek 6254BC: USB ID `04b5:6cde`, family string `6000`,
PCB `P1.06`, FPGA version `0xD001`, driver version `20160531`. Other units and models of the family (for example the
6254BD) are expected to behave the same way but are untested; values that are calibrations of one unit are marked as such.

**Conventions.** Multi-byte fields are little-endian unless stated. Command bytes are hexadecimal. A *frame* is one
sample instant of all enabled channels. An *ADC code* is the raw 8-bit sample value.

## Contents

1. [Hardware](#1-hardware)
2. [USB transport](#2-usb-transport)
3. [Device memory areas](#3-device-memory-areas)
4. [Bulk command reference](#4-bulk-command-reference)
5. [Initialisation](#5-initialisation)
6. [Acquisition](#6-acquisition)
7. [Configuration](#7-configuration)
8. [Triggers](#8-triggers)
9. [Converting samples to volts](#9-converting-samples-to-volts)
10. [Verification status and limitations](#10-verification-status-and-limitations)
- [Appendix A: Measurement data](#appendix-a-measurement-data)
- [Appendix B: Vendor API map](#appendix-b-vendor-api-map)

## 1. Hardware

Component list from third-party teardowns of the 6254BD, which has the same board family. Items marked † were not
confirmed on the BC unit.

| Function | Part |
|---|---|
| USB controller | Cypress CY7C68013A (FX2LP, 8051); runs its firmware from an I2C EEPROM at power-up |
| Firmware EEPROM † | Microchip 24LC64 (64 Kbit) |
| FPGA | Xilinx Spartan-6 XC6SLX16; configuration flash Macronix MX25L4006E (4 Mbit SPI) † |
| ADC | Analog Devices HMCAD1511, 8-bit, up to 1 GS/s, 4 channels |
| Clock synthesizer | Analog Devices ADF4360-7 |
| Front-end control | 74HC595 shift registers (one per channel): attenuator relays, AC/DC relay, bandwidth limit |
| Capture memory | 64 KiB circular RAM behind the FPGA |
| Signal generator † | TI DAC902E on the 6254BD. **The 6254BC used here has no generator output** and its generator calibration areas are blank |

Specification of the 6254BD: 4 channels, 250 MHz, 1 GS/s, 8-bit, 1 MΩ ‖ 25 pF input, edge, pulse, video and alternate
triggers. Third-party reports say the BC and BD share the PCB and differ in EEPROM/FPGA firmware.

## 2. USB transport

### 2.1 Device

**Verified.** USB ID `04b5:6cde` (reported as "ROHM LSI Systems USA" / "DSO Device"), USB 2.0 high speed, one
configuration, one vendor-specific interface (class `0xFF`), bus powered. The device is running its firmware when it
enumerates; no firmware upload is needed (unlike the Hantek 6022).

| Endpoint | Direction | Type | Max packet |
|---|---|---|---|
| `0x02` | OUT | bulk | 512 bytes (64 if not high speed) |
| `0x86` | IN | bulk | 512 bytes (64 if not high speed) |

### 2.2 Vendor control requests

All use `bmRequestType` vendor, `wIndex = 0`.

| bRequest | Dir | wValue | Length | Purpose | Status |
|---|---|---|---|---|---|
| `0xB3` | OUT | 0 | 10 | Transfer-mode preamble, sent before every bulk write (2.3) | Verified |
| `0xB2` | IN | 0 | 10 | Ready flag; byte 0 non-zero = high speed (512-byte packets). The device returns one byte, `01` | Verified |
| `0xA2` | IN | area (section 3) | n | Read a device memory area. The write direction exists and must not be used | Verified (read) |
| `0xE3` | IN | `0x15F0` | 4 | Driver version, 8 BCD digits (`20 16 05 31` = `20160531`) | Verified |
| `0xEA`, `0xE5` | OUT | 0 | 10 | Select the USB or SPI bus path in the vendor driver. `0xEA` as OUT and `0xE5` as a read both **stall** on the 6254BC; not needed | Verified (stalls) |
| `0xEC` / `0xED` | OUT / IN | `0x1700`, `0x1716` | 20, 1 | LAN settings (IP, mask, gateway, port, MAC) and LAN enable flag, for models with a LAN option. `0xED` **stalls** on the 6254BC; `0xEC` (a write) was never sent | Vendor |
| `0xF4` | IN | `0x1F00` | 2 | USB module version. **Stalls** on the 6254BC | Vendor |
| `0xF0` / `0xF1` | OUT / IN | `0x1E00` | 64 | FPGA bitstream flash write/read. **Never write**; not implemented | Vendor |

A stall (`LIBUSB_ERROR_PIPE`) is the device refusing an unsupported request. The control pipe recovers and the scope works
normally afterwards. Control reads longer than 1024 bytes work as a single transfer on the 6254BC (the 1154-byte
calibration read); the vendor driver splits them into 1024-byte chunks.

### 2.3 Bulk commands

**Verified.** Commands are sent on endpoint `0x02`. **Before every bulk write**, the host sends control OUT `0xB3` with
the 10-byte payload `0f 03 03 03 00 00 00 00 00 00` followed by control IN `0xB2`. The vendor library defines other
payloads for other transfer modes (`0f 0a 0a 0a`, `0f 09 09 09`, `0f 01 01 01`, `0f 02 02 02`, `0f 08 08 08`); only
`0f 03 03 03` is used for commands.

Replies arrive on endpoint `0x86` as full packets (512 bytes in high speed). A reply carries its value in the first bytes
and is padded; the host always reads a whole packet. Bulk reads of sample data are multiples of 512 bytes.

### 2.4 Write safety

The EEPROM (FX2 firmware) and the FPGA flash can be corrupted by wrong writes; a bad write can leave the scope unusable
(third-party reports describe bricked units). The library only reads these areas. Back up the factory tables with
`hantek6xx4 dump-calibration` and `dump-amp-calibration` before experimenting.

## 3. Device memory areas

Read with control IN `0xA2`, `wValue` = area address.

| wValue | Length | Content | Status |
|---|---|---|---|
| `0x1580` | 71 | Info string (3.1) | Verified |
| `0x15E0` | 32 | User/driver area: driver version at offset 16, otherwise mostly `0xFF` (3.2) | Verified |
| `0x1600` | 1154 | Position calibration table, 577 words (3.3) | Verified |
| `0x1B00` | 578 | Amplitude (gain trim) table, 289 words (3.4) | Verified |
| `0x1D80` | up to 640 | DDS calibration; blank (`0xFF`) on the 6254BC | Verified (blank) |

### 3.1 Info string

71 bytes; `0xFF` is padding. On the reference unit:

```
44 53 4f ff ff ff 36 30 30 30 ff ff ff ff 50 31   DSO...6000....P1
2e 30 36 ff 44 31 2e 30 30 ff 4d 30 30 37 43 30   .06.D1.00.M007C0
31 31 36 34 43 30 32 31 35 33 32 30 31 37 30 32   1164C02153201702
32 32 32 30 31 37 30 32 32 32 54 30 33 33 46 32   2220170222T033F2
c0 30 31 ff 03 ff ff                              .01....
```

| Bytes | Field | Reference unit | Status |
|---|---|---|---|
| `[6..14)` | device family | `6000` | Verified |
| `[14..20)` | PCB revision | `P1.06` | Verified |
| `[34..42)` | serial-like field | `64C02153` | Vendor |

The string does not contain the model name. The PCB revision digits are parsed into a number (`1.06` becomes 106) that
selects the ADC gain register value (7.2). The remaining fields (`D1.00`, the `M…` block) are unidentified.

### 3.2 User / driver area (`0x15E0`)

32 bytes: `08 bf ff ff ff ff ea ea ff ff ff ff ff ff ff ff 20 16 05 31 ff ff …`. The four bytes at offset 16 are the driver
version (BCD digits `20160531`), also readable with request `0xE3`. The meaning of `08 bf` and `ea ea` is unknown. The
vendor code reads word 1 as a generator calibration factor; it is `0xFFFF` (blank) here, so the vendor library would use
its default.

### 3.3 Position calibration table (`0x1600`)

**Verified.** 577 words: 576 calibration words followed by the validity flag `0xFBCF`. The 576 words are 4 channels ×
12 rows × 12 entries, used to compute the vertical position word (7.6). If the flag is wrong the vendor library
substitutes defaults.

### 3.4 Amplitude table (`0x1B00`)

**Verified.** 289 words: 288 words (4 channels × 12 rows × 6 slots) followed by the flag `0xFACF`. A wrong flag means no
correction. Each value is a gain factor in units of 1/1024 (`0x400` = 1.0). The table is per unit factory data.

- **Row** = vertical range index remapped: 5 for indexes 0 to 5, 7 for 6 to 8, 9 for 9 to 11. Only rows 5, 7 and 9
  (and a few words of row 8) carry data on the reference unit.
- **Slot** 0, 1, 2 = ADC channel mode 1, 2, 4 (7.2). Slots 3 to 5 stay `0x400`.
- Index of a trim: `(channel × 12 + row) × 6 + slot`.

On the reference unit the trims are within a few percent and differ per channel; for the one-channel mode on row 7 they
are CH1 `0x455`, CH2 `0x440`, CH3 `0x445`, CH4 `0x454`. They are close to 1.0 in the four-channel mode. The effect of
the trims is described in 7.7.

## 4. Bulk command reference

All commands are sent on endpoint `0x02` (with the preamble of 2.3). The first byte is the command id, the second is
always `00`.

| Cmd | Bytes | Meaning | Reply | Status |
|---|---|---|---|---|
| `0C` | `0C 00` | Reset | First u16 = FPGA version (`0xD001`); the reply packet repeats `01 d0` | Verified |
| `09` | `09 00` | Hardware version | First u32; all zero on the reference unit | Verified |
| `06` | `06 00` | State word | First u16 (6.1) | Verified |
| `03` | `03 00 m 00` | Start acquisition; bit 0 auto, bit 1 roll, bit 2 stop (6.1) | none | Verified |
| `0D` | `0D 00` | Buffer status (6.2) | u16 trigger address, u16 end address, … | Verified |
| `0E` | `0E 00 lo hi` | Set the byte address where the next readback starts | none | Verified |
| `05` | `05 00 lo hi` | Read back `2 × value` bytes of capture RAM | data packets | Verified |
| `08` | `08 00 00 d0 d1 reg cs 00` | SPI write of the 16-bit word `d1:d0` to register `reg` of chip `cs` (1 relay shift registers, 2 ADF4360-7, 4 HMCAD1511) | none | Verified |
| `0F` | `0F 00 <u32>` | Time base divisor minus one (7.3) | none | Verified |
| `10` | `10 00 <6 bytes> <6 bytes>` | Horizontal trigger position (7.4) | none | Verified |
| `00`, `01`, `02`, `04` | `id 00 lo hi` | Vertical position word of CH1, CH2, CH3, CH4 (7.6) | none | Verified |
| `1E` | `1E 00 …` (26 bytes) | Gain trim (7.7) | none | Verified |
| `07` | `07 00 …` (26 bytes) | Trigger level (8.2) | none | Verified |
| `11` | `11 00 mode slope coupling 00` | Trigger mode (8.3) | none | Verified |
| `12` | `12 00 b2 00 b4 b5` | Trigger source routing (8.1) | none | Verified |
| `13` | `13 00 …` (16 bytes) | Pulse trigger (8.4) | none | Verified |
| `16` | `16 00 …` (16 bytes) | Video trigger (8.5) | none | Vendor |
| `18` | `18 00` | Initialise SDRAM | none | Vendor (not used) |

The channel position id is `00` for CH1 and `1 << (ch − 1)` for CH2 to CH4 (`01`, `02`, `04`). Writes to the position
registers are followed by a 10 ms pause.

## 5. Initialisation

**Verified.** Order used by the library (the same as the vendor library's `dsoInitHard`, minus the `0xEA` bus select that
the 6254BC stalls on):

1. Bulk `0C 00` (reset); read the reply, first u16 = FPGA version.
2. Control IN `0xA2`/`0x1580` for the info string.
3. Control IN `0xA2`/`0x1600` (position calibration; the flag `0xFBCF` must be present) and `0xA2`/`0x1B00`
   (amplitude table; without the flag no trim is applied).
4. ADC and clock synthesizer bring-up by SPI writes (`08 00 00 d0 d1 reg cs 00`), each followed by a pause:

| Chip | Register | Value | Pause after |
|---|---|---|---|
| HMCAD1511 (`cs 4`) | `0x12` | `0x4777` | 2 ms |
| HMCAD1511 | `0x33` | `0x0003` | 2 ms |
| ADF4360-7 (`cs 2`) | `0x30` | `0x0065` | 2 ms |
| ADF4360-7 | `0x0F` | `0xF128` | 2 ms |
| ADF4360-7 | `0x01` | `0x3812` | 15 ms |

The device answers the state and version commands as soon as the reset has been sent. The state word is `0x0000` before
this sequence and `0x8402` after it.

## 6. Acquisition

### 6.1 Start, state, stop

**Verified.** `03 00 m 00` starts an acquisition: `m = 1` auto (free running), `m = 0` normal (wait for a trigger),
`m = 4` stop. In the vendor library bit 0 is auto, bit 1 roll and bit 2 stop after this collection; the roll bit makes no
observable difference on the 6254BC.

The state word (first u16 of the reply to `06 00`):

| Value | Meaning |
|---|---|
| `0x0000` | before initialisation |
| `0x8402` | initialised, idle |
| `0xC100` | acquisition started, capture in progress (also the permanent value during roll streaming) |
| `0xC200` | armed, waiting for a trigger |
| `0xC402` | data ready, no trigger (auto sweep or forced) |
| `0xC403` | data ready and triggered |

Bit 0 is *triggered* and bit 1 is *data ready*. The vendor library also names bit 3 (pack state) and bit 4 (SDRAM
initialised); they were not exercised. In normal sweep with no trigger, bit 1 never gets set.

A capture takes the time of the pre-trigger fill plus the post-trigger part of the window, so the host polls the state
until bit 1 is set (a 20 ms pause between polls is plenty) and stops with `03 00 04 00` on timeout.

### 6.2 Buffer status and trigger-aligned readback

**Verified.** `0D 00` returns, among other fields: `u16 @ 0` = byte address of the trigger edge in the 64 KiB capture RAM
(only valid after a triggered capture, within about 20 bytes of the edge), `u16 @ 2` = end address (the write pointer),
byte `@ 4` (`06` or `07`), `u16 @ 6` = `0x5555`, `u16 @ 8` = copy of `u16 @ 0`.

Reading a window of `B` samples per channel after the state shows *data ready*:

1. `0D 00`; `end = u16 @ 2`.
2. Triggered: `start = (end − B × stride) mod 0x10000`, rounded down to a multiple of 8, where `stride` is the bytes per
   frame (6.3). Not triggered (auto sweep): `start = 0`.
3. `0E 00 start_lo start_hi`.
4. `05 00 w_lo w_hi` with `w = B × stride / 2`.
5. Read `B × stride` bytes in 512-byte packets.

The window then contains the trigger edge at the configured horizontal position (7.4), to within a few samples (section
10). The documented depth is `B = 4096` samples per channel; the RAM holds 64 KiB.

### 6.3 Sample data layout

**Verified.** Samples are unsigned 8-bit, one byte per enabled channel per frame:

| Enabled channels `n` | Stride (bytes per frame) | Order within a frame |
|---|---|---|
| 1 | 1 | the channel |
| 2 | 2 | enabled channels in ascending order |
| 3 or 4 | 4 | CH1, CH2, CH3, CH4 (the unused channel of a 3-channel capture holds a constant) |

The stride follows the number of enabled channels, not the ADC channel mode of 7.2. The 64 KiB RAM holds data from older
captures; only the first `B × stride` bytes of a fresh acquisition are valid. The first 16 frames of an auto-sweep
capture read from address 0 may be stale and the library drops them. A 1 kHz square wave has the same period in samples
in 1-, 2- and 4-channel modes at the same time base.

### 6.4 Continuous streaming (scan and roll)

**Verified.** In the vendor software, scan (time base index 25 and slower) and roll (index 27 and slower) are display
modes; the hardware behaviour is the same. After `03 00 m 00` the FPGA keeps writing frames into the 64 KiB RAM, wrapping
at `0x10000`, and the host reads what is new:

1. `0D 00`; `end = u16 @ 2` is the write pointer.
2. New bytes = `(end − last) mod 0x10000`, rounded down to whole frames (and to an even length).
3. `0E 00 last_lo last_hi`, `05 00 w_lo w_hi` with `w = bytes / 2`, then read the bytes. Reading across the wrap works.
4. Remember `end` as `last`.

The pointer advances in steps of 8 bytes at `stride / sample_interval` bytes per second, so at slow time bases data
arrives in bursts of 8 bytes (one burst every 0.64 s at 20 s per division with one channel). The state word stays
`0xC100`. The first 11 to 16 frames after the start are stale and are discarded. If the host does not read before the
RAM has wrapped, data is lost and the pointer alone cannot reveal it; the library estimates that from the elapsed time.
The vendor's `dsoHTSetSampleRate` ignores its scan/roll format argument, and `dsoHTOpenRollMode`, `dsoHTCloseRollMode`
and `dsoHTStartRoll` do nothing.

## 7. Configuration

### 7.1 Order of operations

**Verified.** The library applies a configuration in the order of the vendor software:

1. ADC register `0x55` (7.2)
2. ADC channel-mode registers (7.2), timebase `0F` (7.3), horizontal position `10` (7.4)
3. Relays (7.5), twice with pauses, then the ADC channel-mode registers again
4. Trigger source `12` (8.1)
5. Vertical position of each channel (7.6), 10 ms pause each
6. Trigger level `07` (8.2), trigger mode `11` (8.3) and the pulse or video packet (8.4, 8.5)
7. Gain trim `1E` (7.7)

The gain trim must come last; sent earlier it has no effect.

### 7.2 ADC channel mode and gain registers

**Verified** for 1, 2 and 4 channels at time base indexes 3, 6, 7, 8 and 15.

The ADC runs in one of three channel modes `k`, with the sample rates of the manual's table: single channel 1 GS/s, dual
500 MS/s, quad 250 MS/s. With `n` enabled channels and time base index `t` (0 = 2 ns):

| n | k |
|---|---|
| 3 or 4 | 4 |
| 2 | 2 if `t ≤ 7`, else 4 |
| 1 | 1 if `t < 7`, 2 if `t = 7`, else 4 |

The per-channel gain code `g` from the vertical range index `v`:

| v | 0 | 1 | 2 | 3, 6, 9 | 4, 7, 10 | 5, 8, 11 |
|---|---|---|---|---|---|---|
| g | `0xD` | `0xA` | `0x7` | `0x5` | `0x2` | `0x0` |

HMCAD1511 registers (SPI `cs 4`), written 3 ms apart in this order:

| Reg | Value |
|---|---|
| `0x3A` | k=4: `0x0810`. n=1, t<7: `0x1010`, `0x0808`, `0x0404`, `0x0202` for CH1 to CH4. n=1, t=7: CH1 `0x1010`, CH2 `0x0404`, CH3 `0x0404`, CH4 `0x1010`. n=2, t≤7: CH1+2 `0x1010`, 1+3 `0x1010`, 2+3 `0x0808`, 1+4 `0x1010`, 2+4 `0x0808`, 3+4 `0x0404` |
| `0x3B` | k=4: `0x0204`. n=1, t<7: same as `0x3A`. n=1, t=7: CH1 `0x0808`, CH2 `0x0808`, CH3 `0x0202`, CH4 `0x0202`. n=2, t≤7: 1+2 `0x0808`, 1+3 `0x0404`, 2+3 `0x0404`, 1+4 `0x0202`, 2+4 `0x0202`, 3+4 `0x0202` |
| `0x0F` | `0x0000` |
| `0x31` | k=4: `0x0204`; k=2: `0x0102`; k=1: `0x0001` |
| `0x2A` (k=4) | `g4<<12 \| g3<<8 \| g2<<4 \| g1` |
| `0x2B` (k=1, 2) | build `word` (n=1: `g × 0x1111` of the enabled channel; n=2: low byte `g(first) × 0x11`, high byte `g(second) × 0x11`); then data bytes `d0 = ((hi ^ lo) & 0xF) ^ hi`, `d1 = lo & 0xF` |

Once per configuration, before these: register `0x55` (SPI `cs 4`) selects a gain by the PCB revision number parsed from
the info string. Revision 105: `0x37` (k=4), `0x0A` (k=2), `0x00` (k=1). Revision 106: `0x3F` for every mode (verified on
the reference unit, PCB 1.06). Other revisions: `0x3F`, `0x30`, `0x19`.

### 7.3 Time base

**Verified.** There are 36 time bases from 2 ns to 1000 s per division in a 1-2-5 sequence, 250 samples per division.
`0F 00 <u32>` carries `divisor − 1`. The sample interval is `ns_per_division / 250` from index 8 on; for indexes 0 to 7
it is the ADC mode interval (1 ns for one channel, 2 ns for two, 4 ns for three or four; index 7: 2, 2, 4 ns), set by
the ADC registers of 7.2 and not by the divisor.

| Idx | Time base | `0F` payload (divisor − 1) | Sample interval |
|---|---|---|---|
| 0 | 2ns | 0 | 1 / 2 / 4 ns |
| 1 | 5ns | 0 | 1 / 2 / 4 ns |
| 2 | 10ns | 0 | 1 / 2 / 4 ns |
| 3 | 20ns | 0 | 1 / 2 / 4 ns |
| 4 | 50ns | 0 | 1 / 2 / 4 ns |
| 5 | 100ns | 0 | 1 / 2 / 4 ns |
| 6 | 200ns | 0 | 1 / 2 / 4 ns |
| 7 | 500ns | 0 | 2 / 2 / 4 ns |
| 8 | 1us | 0 | 4 ns |
| 9 | 2us | 0 | 8 ns |
| 10 | 5us | 1 | 20 ns |
| 11 | 10us | 4 | 40 ns |
| 12 | 20us | 9 | 80 ns |
| 13 | 50us | 24 | 200 ns |
| 14 | 100us | 49 | 400 ns |
| 15 | 200us | 99 | 800 ns |
| 16 | 500us | 249 | 2 us |
| 17 | 1ms | 499 | 4 us |
| 18 | 2ms | 999 | 8 us |
| 19 | 5ms | 2499 | 20 us |
| 20 | 10ms | 4999 | 40 us |
| 21 | 20ms | 9999 | 80 us |
| 22 | 50ms | 24999 | 200 us |
| 23 | 100ms | 49999 | 400 us |
| 24 | 200ms | 99999 | 800 us |
| 25 | 500ms | 249999 | 2 ms |
| 26 | 1s | 499999 | 4 ms |
| 27 | 2s | 999999 | 8 ms |
| 28 | 5s | 2499999 | 20 ms |
| 29 | 10s | 4999999 | 40 ms |
| 30 | 20s | 9999999 | 80 ms |
| 31 | 50s | 24999999 | 200 ms |
| 32 | 100s | 49999999 | 400 ms |
| 33 | 200s | 99999999 | 800 ms |
| 34 | 500s | 249999999 | 2 s |
| 35 | 1000s | 499999999 | 4 s |

Indexes 8 and 9 send divisor 1 as well; their 4 ns and 8 ns intervals follow from the nominal 250 samples per division
and were confirmed by the rise-time measurement of Appendix A.6. The interval was measured exactly (to 1 ppm against a
1 kHz calibrator) from index 13 to 22.

### 7.4 Horizontal trigger position

**Verified** at 10 %, 50 % and 90 %. `10 00 <first: 6 bytes> <second: 6 bytes>`, both values in units of 8 ns (a
125 MHz counter), computed from the window length `B` (samples), the trigger position `h` in percent, the sample
interval `f` in ns and the FPGA version (latency `lat`: 246 for FPGA versions `0xA000`, `0xA00A`, `0xA00B`, otherwise
230):

```
pre    = trunc(h × B / 100)
first  = trunc((lat + (pre + 100) × f) / 8)
second = trunc(((B − pre + 8) × f) / 8)
```

If `f < 200` ns each result is rounded up when it has a fraction. Example: `B = 4096`, `h = 50`, `f = 800` gives
`first = 214828`, `second = 205600`, which are the constants hard-coded in two open-source drivers. The trigger edge lands
within a few samples of the requested position (section 10).

### 7.5 Front-end relays

**Verified** for all 12 ranges, the bandwidth limit and AC coupling. SPI to the shift registers (`cs 1`):
`08 00 c1 c2 c3 c4 01 00`, one byte per channel:

| Bit | Meaning |
|---|---|
| 7 | bandwidth limit on |
| 6 | range index > 8 (above 1 V per division) |
| 5 | range index ≤ 8 |
| 4 | range index > 5 (above 100 mV per division) |
| 3 | range index ≤ 5 |
| 2 | DC coupling (cleared for AC) |
| 1 | always 1 |
| 0 | 0 |

The relays are latching: send the message, wait 4 ms, send it again with each byte `AND 0x86` (relay drive bits cleared,
bandwidth, DC and base bits kept), wait 50 ms.

### 7.6 Vertical position

**Verified** for all 12 ranges in modes 1 and 4. The position word `W` of a channel is computed from the calibration table
`T` (3.3) and the position `p` (0 to 255, 128 centred):

```
row = 5 if v ≤ 5; 8 if v is 6 to 8; 11 if v is 9 to 11
m   = 4 (k=4), 2 (k=2), 0 (k=1)
idx = m + 12 × (12 × ch + row);  a = T[idx], b = T[idx + 1]
if v is 5, 8 or 11:  lo = a,  hi = b
else:  avg = (a + b + 1) >> 1;  mul = 0.02 (v=0), 0.05 (v=1), 0.1 (v=2), 0.2 (otherwise)
       d = floor((b − avg) × mul + 0.5);  lo = avg − d;  hi = avg + d
W = trunc((hi − lo) × p / 255 + lo)
```

`W` is sent as `id 00 W_lo W_hi` (id as in section 4). The resulting zero level in ADC codes is given in section 9.

### 7.7 Gain trim (`1E`)

**Verified.** 26 bytes applying the amplitude table (3.4) to the FPGA:

| Bytes | Content |
|---|---|
| 0..1 | `1e 00` |
| 2..9 | four trims `t0..t3` (u16) |
| 10..25 | for each trim `ti ≠ 0x400`, a 4-byte offset word at `10 + 4i`: `trunc(p' × (1024 − ti) + 0.5)` (add `0x2000000` if negative), with `p' = trunc((position − 128) × 0.78125 + 128.5)`; zero otherwise |

Which channel supplies the trim and the position for the four slots depends on the number of enabled channels `n`:
`n = 1`: the enabled channel for all four; `n = 2`: the first enabled channel's trim for all four slots and positions
`[first, first, second, second]`; `n = 3` or `4`: each channel its own. The trim is looked up with the channel mode `k` of
7.2 as the slot, so one channel at a time base index above 7 uses slot 2.

The FPGA multiplies the samples by `t / 1024` around the zero level (verified: a trim of `0x440` raised a signal above
zero by 1.06 while the zero stayed at code 128). The setting persists across acquisitions. The table values correct
per-channel gain differences of the analog front end; they are not volts-per-count constants.

## 8. Triggers

### 8.1 Trigger source routing

**Verified** for the tested channel and time base combinations. `12 00 b2 00 b4 b5` with the time base index `t`,
channel mask `m` (CH1 = bit 0), source channel `s`, peak-detect flag `p` (always 0 here), `n` enabled channels and
`uv = m >> s`:

```
b2 = (p << 6) | (m << 2) | x
b4 = y & 3
b5 = ((~uv & 1) << 2) | z
n = 1:    x = 0 (t<7), 1 (t=7), 2 (t=8), 3 (t>8);  y = 3 − x
n = 2:    x = 0 (t<8), 1 (t=8), 2 (t>8);           y = 2 − x
n = 3, 4: x = (t > 8);                              y = 1 − x
z = 0 (k=1);  k=2: n=1 ? (m is CH1 or CH3 ? 0 : 1) : (uv == 1);  k=4: s
```

### 8.2 Trigger level

**Verified.** The level is an ADC code (0 to `0xE4`) compared with the raw samples; use the zero model of section 9 to
convert a voltage. `07 00` followed by four groups `hi hi lo lo` and eight copies of `mid`, where `mid` is the level,
`hi = min(mid + 4, 0xE4)` and `lo = max(mid − 4, 0)` (a ±4 code hysteresis band). The vendor API takes a position
`nPos` (0 to 255) and maps it with `mid = trunc(nPos × 200 / 256 + 28.5)`.

### 8.3 Trigger mode

**Verified.** `11 00 mode slope coupling 00`: mode `0` edge, `1` pulse, `2` video (vendor: CAN `0x0B`, LIN `0x0A`, UART
`0x09`, SPI `0x0C`, I2C `0x0D`, force `0x80`; not implemented); slope `0` rising (for pulses: positive), `1` falling
(negative); coupling `0` DC (the vendor API also defines AC, low-frequency, high-frequency and noise rejection, untested).
The edge trigger fires on the configured slope when the signal crosses the level.

### 8.4 Pulse trigger

**Verified** with the 1 kHz calibrator (500 µs pulses), after the mode command with mode `1`. `13 00 <a: u32> <1.05a: u32>
<0.95a: u32> <cond << 1> 00`, 16 bytes, with `a = trunc(width_ns × 0.125)` (units of 8 ns) and `cond` 0 equal, 1 not
equal, 2 greater, 3 less. The accepted width range is 10 ns to 10 s. "Equal" accepts widths within ±5 %.

The vendor header documents the width in units of 5 ns, which would give `width_ns / 40`; with that the hardware saw
pulses five times too wide, so the argument is effectively in nanoseconds. Test results are in Appendix A.5. The capture is
aligned to the leading edge of the pulse, like an edge trigger.

### 8.5 Video trigger

**Vendor**, not verified with a video signal (the device accepts it, never triggers on the calibrator, and works normally
afterwards). After the mode command with mode `2`: `16 00 <c_lo> <b3> <level+2> <level−2> 5b 0e 3b 0d 4b 02 2c 01 <src> 32`,
16 bytes, where

- `counter` is the hardware line counter of the 1-based line number, `c_lo` its low byte;
- `b3 = ((positive << 3 | sync) << 3) | ((counter >> 8) & 3)`; `positive` = 1 for positive sync polarity; `sync` = 0 all
  lines, 1 line number, 2 odd field, 3 even field, 4 all fields (the line number is 1 for the modes that do not use one);
- `level` is the trigger level as for 8.2; the bytes `5b 0e 3b 0d 4b 02 2c 01` are fixed timing constants copied from the
  vendor code; `src` (logic trigger source) is 0.

The line counter mapping (16-bit arithmetic):

| Standard | Line `n` | Counter |
|---|---|---|
| PAL / SECAM | 0 to 5 | `2n + 0x134` |
| | 6 to 310 (`0x136`) | `n + 0x13E` |
| | 311 to 318 (`0x13E`) | `2n − 0x125` |
| | 319 to 622 (`0x26E`) | `n − 0x13F` |
| | 623 to 625 (`0x271`) | `2n − 0x3AE` |
| NTSC | 0 to 9 | `2n + 0xFC` |
| | 10 to 263 (`0x107`) | `n + 0x106` |
| | 264 to 271 (`0x10F`) | `2n − 1` |
| | 272 to 525 (`0x20D`) | `n − 0x110` |

## 9. Converting samples to volts

**Verified** against a 0.994 V (multimeter) and a 0.298 V DC source, from 50 mV to 1 V per division. The vendor SDK demos
assume `volts = (code − (255 − position)) × 8 × V/div / 255`, that is 32 counts per division and a zero code of
`255 − position`. That does not hold on the 6254BC: it reads the 1 kHz calibrator as 1.5 V where the true output is about
1.91 V. Use instead:

```
volts = (code − zero) × (V/div) / counts_per_div
```

Counts per division and zero code depend on the ADC channel mode `k` (7.2). The position control always spans 8 divisions,
so zero at position 0 and 255 scales with the counts per division:

| k | counts per division | zero at position 0 / 255 | same, ranges 50 mV, 500 mV, 5 V (indexes 4, 7, 10) |
|---|---|---|---|
| 4 | 24.15 | 31 / 224 | 89 / 166 |
| 2 | about 21.6 | about 39 / 216 | 92 / 162 |
| 1 | about 21.1 | about 43 / 212 | 99 / 156 |

The zero code is `lo + position × (hi − lo) / 255` with these end values; position 128 is code 128 in every mode. The
scale holds with the factory gain trim of 7.7 applied. Measurement data is in Appendix A.2 and A.3.

Limits: the values come from one unit; zero levels vary by a few counts between ranges and between channels (a shorted CH4
sat 2 counts above a shorted CH1 at 200 mV per division), so volts can be off by 2 to 3 counts (up to about 5 % of a
division-scale reading) at extreme positions in the fast modes; ranges below 50 mV and above 1 V per division were not
checked against a reference. An open input is not a zero reference: it picks up signals from neighbouring channels (12
counts peak-to-peak at 100 mV per division from a calibrator on the next input).

## 10. Verification status and limitations

| Feature | Status |
|---|---|
| USB transport, initialisation, identification, calibration tables | Verified |
| 1, 2, 3, 4 channel acquisition, data layout | Verified (3 channels: layout only) |
| Time bases | Tested at indexes 0 to 22 (period for 13 to 22, edge rise time for 0 to 12) and at 24, 25, 27 and 30 (streaming rates); the others share the same divisor table but were not measured |
| 12 vertical ranges, relays, bandwidth limit, AC/DC coupling | Verified |
| Edge trigger (level, slope, position, auto and normal sweep) | Verified |
| Pulse trigger | Verified (500 µs pulses, all conditions, both polarities) |
| Video trigger | Vendor only (no video signal) |
| Scan/roll streaming | Verified |
| Gain trim table and zero model | Verified on one unit |
| Volts conversion | Verified from 50 mV to 1 V per division |
| Serial-bus triggers (CAN, LIN, UART, SPI, I2C), peak detect, alternate trigger | Not implemented |
| Signal generator (DDS) | Absent on the 6254BC; not implemented |
| LAN / USB-module requests (`0xEC`, `0xED`, `0xF4`) | Stall on the 6254BC |

Known limitations of the library and open items:

- **Trigger position accuracy.** The trigger edge lands within about ±6 samples of the requested position, and the
  offset depends on the number of enabled channels and the time base (at 200 µs per division about +5, −2 and −6
  samples for 1, 2 and 4 channels, Appendix A.4). The hardware latency is not modelled.
- **Window arithmetic** (6.2) was verified for 1, 2 and 4 channels at 4096 samples; other window lengths are untested.
- **Volts** scale and zero model come from one unit and a few DC levels (section 9).
- **Roll streaming** cannot detect lost data from the pointer; the library infers overruns from the elapsed time.
- The BC and BD share the PCB according to third-party reports, but only a BC was tested.

## Appendix A: Measurement data

### A.1 Test setup

A single Hantek 6254BC on a Linux host through `rusb`/libusb. Signal sources: the scope's 1 kHz probe-compensation
output (about 1.91 V peak-to-peak, nominally 2 V), a laboratory DC source read with a multimeter (0.994 V and 0.298 V,
the latter with a load to stabilise it), and shorted inputs as zero references.

### A.2 Counts per division

DC source on CH2, shorted input on another channel as zero (medians or means of two to three captures). Counts per
division is `(CH2 − zero) / (volts / (V/div))`.

| Source | Range | Mode k=4 | Mode k=2 | Mode k=1 |
|---|---|---|---|---|
| 0.994 V | 200 mV, 500 mV, 1 V per div | 24.1 at each | (500 mV, 1 V) 21.5 to 21.6 | (500 mV, 1 V) 21.1 |
| 0.298 V | 50 mV per div (position 0) | 24.0 | 21.5 to 21.6 | 21.3 |
| 0.298 V | 100 mV per div | 23.8 | 21.8 | 20.5 to 20.8 |

At 2 V per division and above the signal is only a few counts, so quantisation dominates. The library's volts read 0.294 to
0.301 V for 0.298 V and 0.99 to 1.00 V for 0.994 V.

### A.3 Zero level of a shorted input

Mean ADC code at position 0 / 128 / 255, shorted input, four-channel mode (modes 1 and 2 follow the end values of
section 9):

| Idx | Range | Pos 0 | Pos 128 | Pos 255 | Noise s.d. (codes) |
|---|---|---|---|---|---|
| 0 | 2 mV | 38 to 39 | 136 | 230 to 233 | 19 to 24 |
| 1 | 5 mV | 33.8 | 131 to 133 | 228 | 8.6 |
| 2 | 10 mV | 32.7 to 33.3 | 129 to 130 | 226 | 4.1 |
| 3 | 20 mV | 31.5 to 32.0 | 128.9 | 224.8 | 2.1 |
| 4 | 50 mV | **89.3** | 128.0 | **166.6** | 0.9 |
| 5 | 100 mV | 31.1 | 128.1 | 224.4 | 0.5 |
| 6 | 200 mV | 31.5 | 128.3 | 224.4 | 0.5 |
| 7 | 500 mV | **89.0** | 128.0 | **166.1** | ≤ 0.05 |
| 8 | 1 V | 31.0 | 128.0 | 224.0 | 0 |
| 9 | 2 V | 31.7 | 128.2 | 224.5 | 0.4 |
| 10 | 5 V | **89.0** | 128.0 | **166.1** | 0 |
| 11 | 10 V | 31.0 | 128.0 | 224.0 | 0 |

The 2 mV and 5 mV rows are offset by a few codes (residual amplifier offset; the calibration table has no separate row
for them). On the 20 mV range and below the readings are dominated by input noise. The bandwidth limit reduces that
noise: the standard deviation falls from 21.4 to 9.8 codes at 2 mV, 8.5 to 4.2 at 5 mV and 2.1 to 1.0 at 20 mV.

### A.4 Trigger position

Rising edge of the 1 kHz calibrator, 4096-sample window, 200 µs per division (800 ns per sample), level mid-way between the
plateaus. Offset of the observed edge from the requested position, in samples:

| Enabled channels | Rising | Falling |
|---|---|---|
| 1 | +6 | +4 |
| 2 | −1 to −2 | −3 to −4 |
| 4 | −5 | −6 |

At 100 µs per division with one channel the edge was at +0 to +1, so the offset is not constant across time bases.

At 10 %, 50 % and 90 % horizontal position with four channels, the edge lay 5 samples (about 4 µs) before the ideal position
with a jitter of ±1 to 2 samples.

### A.5 Pulse trigger

Calibrator on CH4 (positive and negative pulses of 500 µs), normal sweep. "T" = triggered, "–" = timed out:

| Condition and width | Result | | Condition and width | Result |
|---|---|---|---|---|
| equal 500 µs, positive / negative | T / T | | greater 400 µs | T |
| equal 480 µs, 520 µs | T, T | | greater 600 µs | – |
| equal 450 µs, 560 µs | –, – | | less 400 µs | – |
| equal 200 µs, 1 ms | –, – | | less 600 µs | T |
| not equal 200 µs | T | | not equal 500 µs | – |

With the vendor-documented 5 ns unit the greater-than threshold sat between 2.4 and 2.5 ms for the 500 µs pulse at
20 µs, 200 µs and 1 ms per division, i.e. exactly 500 µs ÷ 0.2: the width unit is 8 ns for an argument in nanoseconds.

### A.6 Sample clock

| Check | Result |
|---|---|
| Mean calibrator period on the time axis, 200 µs to 50 ms per division, up to 818 periods | 1000.000 µs (±0.001 %) |
| Half period at 50 µs per division (index 13) | 500.0 µs |
| 10 to 90 % rise time of the calibrator edge, 20 µs per division down to 2 ns per division | 240 to 256 ns at every setting, 4 samples at 80 ns, 31 at 8 ns, 63 at 4 ns and about 250 at 1 ns |

### A.7 Roll streaming

Calibrator aliased into a repeating 5-sample pattern (200 ms per division, 0.8 ms per sample). Four channels over 22 s
(1.68 buffer wraps): 27 582 samples per channel, the pattern holds across all polls and the wrap; the only deviations are
single-sample flips about every 3000 samples where an edge sits on the threshold because the sample clock drifts slowly
against the calibrator. Rates: 499 samples/s at 500 ms per division, 123 at 2 s (expected 125), 1247 at 200 ms
(expected 1250).

## Appendix B: Vendor API map

The vendor SDK (`HTHardDll`) exports the functions below; the table shows where the library implements them.

| Vendor function | Command or area | Section |
|---|---|---|
| `dsoInitHard` | initialisation | 5 |
| `dsoHTGetState`, `dsoHTStartCollectData` | `06`, `03` | 6.1 |
| `dsoHTGetData` | `0D`, `0E`, `05` | 6.2 |
| `dsoHTGetScanData`, `dsoHTGetRollData` | `0D`, `0E`, `05` | 6.4 |
| `dsoHTADCCHModGain` | ADC register `0x55` | 7.2 |
| `dsoHTSetSampleRate` | ADC registers, `0F`, `10` | 7.2 to 7.4 |
| `dsoHTSetCHAndTrigger` | relays, ADC registers, `12` | 7.5, 8.1 |
| `dsoHTSetCHPos` | `00`/`01`/`02`/`04` | 7.6 |
| `dsoHTSetAmpCalibrate` | `1E` | 7.7 |
| `dsoHTSetVTriggerLevel` | `07` | 8.2 |
| `dsoHTSetTrigerMode` | `11` | 8.3 |
| `dsoHTSetPulseTriger`, `dsoHTSetVideoTriger` | `13`, `16` | 8.4, 8.5 |
| `dsoHTReadCalibrationData`, `dsoHTRDAmpCali` | `0xA2/0x1600`, `0xA2/0x1B00` | 3.3, 3.4 |
| `dsoGetDriverVersion` | `0xE3/0x15F0` | 2.2 |
| `dsoGetHardInfo`, `dsoSetHardInfo` | `0xA2/0x1580` (the library only reads) | 3.1 |
| `dsoSetUSBBus`, `dsoSetSPIBus` | `0xEA`, `0xE5` | 2.2 |
| `dsoUSBModeSet/GetIPAddr`, `dsoSet/GetLANEnable`, `dsoGetUSBModulVersion` | `0xEC`, `0xED`, `0xF4` | 2.2 |
| `dsoWriteFlash`, `dsoReadFlash` | `0xF0`, `0xF1` | 2.2 (not implemented) |
| `dds*` (generator) | not on the 6254BC | 1 |
