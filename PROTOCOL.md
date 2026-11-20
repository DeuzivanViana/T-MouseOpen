# HID protocol — receiver 04F3:026F (Titorion YDX-GM009 / "MSF-001 Pro Gaming Mouse")

Confidence levels: **[HW]** tested on hardware · **[CAP]** seen in a USB capture · **[RE]** read from `GamingMS.exe` v2.1.5 (not tested).

## Interfaces
- Interface 0: standard mouse, endpoint 0x81, 7-byte input report.
- Interface 1: vendor-defined control interface, endpoint 0x82, 252-byte report descriptor. Commands go over control transfers (SET/GET_REPORT, Feature).

## Report 0x06: commands (Feature, 32 bytes) [RE + CAP + HW]
| byte | content |
|---|---|
| 0 | `0x06` (report ID) |
| 1 | command code |
| 2 | sequence counter: the software starts at **1** (`0x40a51c`) and increments it for every frame. **A frame with sequence 0 is ignored by the firmware** (observed on hardware: the first DPI apply after launch failed when it started at 0), so 0 must never be sent |
| 3 | `(part index << 4) | total parts`. Single-part command = `0x01`; DPI = `0x02` and `0x12` |
| 4 | checksum = sum of all 32 bytes with this byte zeroed, `& 0xFF` |
| 5… | data |

Ack: after each SET, the software GETs Feature 0x06 every 60 ms (up to 3 tries) until `buf[1] != 0`. `06 01` = OK. Read too early, it returns `06 00`. [RE + HW]

Captured frames the generator reproduces exactly: `06 04 09 01 18 04` (250 Hz) and `06 04 0a 01 17 02` (500 Hz).

## Command 0x04: polling rate
Code (same as status byte 3): 125→`08` · 250→`04` **[HW]** · 500→`02` **[HW]** · 1000→`01` · (2000→`11`, 4000→`12`, 8000→`14`: the software only offers these depending on status byte 8).

## Command 0x02: DPI [RE + HW]
Two parts (`0x437631` and `0x437830`), 18 data bytes each:
- Part 0 (`buf[3]=0x02`): `[active stage] [mask] [X0 lo hi] … [X6 lo hi] [00 00]`
- Part 1 (`buf[3]=0x12`): `[active stage] [mask] [Y0 lo hi] … [Y6 lo hi] [00 00]`
- Active stage: zero-based. Mask: bit i = stage i enabled (7 stages: bits 0–6; bit 7 comes from an 8th checkbox in the official UI, meaning unknown).
- **The mouse DPI button only cycles through the stages enabled in the mask.** With mask `0x0F` and the 4th stage at 2400, DPI+ stops at 2400 (observed on hardware; not a firmware limit).
- The per-stage code depends on the **sensor** (status byte 9). **PAW3311 (`0x11`)**: code = index, DPI = (index+1)×100 → **100 to 12000, step 100** (branch `0x3311` at `0x408aab`; the UI displays value×50).
- Other sensors have their own tables (the software handles 3104, 3311, 3325, 3327, 3212, 3335, 3360, 3370, 3389, 3395, 3950, 3399).

## Report 0x04: status (Input) [CAP + RE]
Decoded the same way the official software does (`0x433937…`). The software only **listens** to this report (ReadFile); there is no command to read back the configuration.
| byte | meaning |
|---|---|
| 2 | active DPI stage (zero-based) [RE + HW] |
| 3 | polling code [HW] |
| 4 | battery % [RE + HW: 30% matched] |
| 5 | 0 = on battery, 1 = charging, 2 = fully charged (the software forces 100%) [RE] |
| 7 | profile (0 = none) [RE] |
| 8 | receiver tier: `02` → up to 1000 Hz; `04` → +2000; `08` → +4000 [RE] |
| 9 | sensor: `0x11` = PAW3311 [RE + CAP] |

Arrives every ~5 s and ~20 ms after each command. In the startup capture it arrived ~50 ms after the GET of report 0x05. Hypothesis: while the mouse is asleep the receiver sends no status.

## Report 0x05: info (Feature, read at startup) [CAP]
`05 31 30 31 30 01 01 …` → ASCII "1010" (likely the firmware version).

## Other report 0x06 commands [RE, meaning not confirmed]
| cmd | data | hypothesis |
|---|---|---|
| 0x03 | len 1 | per-stage DPI colors? (RGB block built right after the DPI table) |
| 0x05 | 1 byte: 1 if the profile field equals 1, otherwise 2 | lift-off distance (profile `LiftDetectionValue=02`) |
| 0x06 | several bytes | ? |
| 0x07, 0x08 | ? | lighting? |
| 0x09 | 1 byte, raw profile value | angle snapping? (profile `AngleSnapValue=00`) |
| 0x0F | ? | ? |
| 0x10 / 0x11 | transfer in 25-byte chunks | macro upload — **do not use yet** |

## Do not touch
- Report **0xBF** (136 bytes, loop over 0x2B000 bytes in 128-byte chunks) and **0xBC**: firmware update. The executable embeds firmware images (offsets 0x800000 and 0x844000).

## `.psf` profile (config\defaultN.psf, UTF-16)
Holds values in device format, e.g. `PollingRateValue=020000`, `LiftDetectionValue=02`, `AngleSnapValue=00`, `DPIOffOnFlagValue=3F`, `CurrentDPILevelValue=02`, `XYSetting1..8Value` (DPI), `DPILevelIndicationNValuer` (RGB color per stage), `KeyNValue` (buttons), `Lighting1*`.
