# A/321 — System Discovery and Signaling (the "Bootstrap")

Source: ATSC A/321:2026-06, "System Discovery and Signaling", 11 June 2026 (34 pages). See
[`README.md`](README.md) for provenance and exact download URL.

Scope of this document: the bootstrap is a **physical-layer** signal — a fixed-format prefix
(sampling rate, bandwidth, FFT size, symbol length are all constant regardless of the version
signaled) that precedes every physical layer frame and lets a receiver discover the frame's
signal type before it can decode anything else. It is included here because it is the entry
point ATSC 3.0 tuning depends on, and issue #750 groups it with the ROUTE/MMT crate as the
"system discovery" half of the ATSC 3.0 signalling stack. **It is not a byte/octet-oriented
wire structure** in the way ROUTE/LCT or the LLS/SLS XML fragments are — its "syntax" is a set
of a handful of signaling bits per bootstrap OFDM symbol, recovered by the receiver's physical
layer (ZC-sequence root selection + PN-sequence-seed selection + explicit signaling bits per
symbol), not by parsing a byte stream. A `no_std` crate consuming this document would model it
as a small set of enums/bitfields (major/minor version, per-symbol signaling fields), not as a
`Parse`/`Serialize` byte-buffer type in the same sense as the rest of this workspace — there is
no octet-aligned buffer to hand it; the values arrive already demodulated from the PHY layer.

## 1. Central concepts (§4.2, informative + normative mix)

- **Versioning**: `bootstrap_major_version` (signaled by the Zadoff-Chu sequence *root*) and
  `bootstrap_minor_version` (signaled by the pseudo-noise sequence *seed*), each conventionally
  written `major.minor` (e.g. "bootstrap version 0.0"). A major version identifies a signal
  type (a different underlying physical-layer technology); a minor version identifies an
  evolution/reconfiguration within the same major signal type. **ATSC 3.0 itself is identified
  by bootstrap version 0.0** (`bootstrap_major_version = 0`, `bootstrap_minor_version = 0`),
  per §7.3.
- **Scalability**: the number of signaling bits carried per bootstrap symbol has a defined
  maximum for a given major/minor version and can be increased (as a backward-compatible
  change) up to that maximum when the minor version is incremented within the same major
  version.
- **Extensibility**: the bootstrap's total duration is extensible in whole symbol periods; a
  final symbol is signaled by a 180 degree phase inversion relative to the preceding symbol.
  Undefined/reserved signaling values are expected to cause the receiver to discard the
  bootstrap.
- **Universal accommodation**: the bootstrap prefix identifies which post-bootstrap signal
  type/technology follows, so receivers unsupportive of a signaled type can discard those
  frames without attempting to decode them.

## 2. Fixed signal dimensions (§5.1, normative)

These values do **not** vary with version:

| Parameter | Value |
|---|---|
| Sampling rate | 6.144 Msamples/second |
| Bandwidth | 4.5 MHz |
| FFT size (`N_FFT`) | 2048 |
| Subcarrier spacing | 3 kHz |
| Bootstrap symbol duration | 500 microseconds |

The number of bootstrap symbols (`N_S`) is not fixed — it depends on the major/minor version in
use — and must not be assumed by a receiver.

## 3. Bootstrap major version 0 (§6.1) — ATSC 3.0's own bootstrap

- ZC sequence root `q` = 137 when `bootstrap_major_version = 0` (§6.1).
- `N_S >= 4` (including the initial synchronization symbol) for all minor versions under major
  version 0 (§6.1.1).
- Minor version is signaled by the PN sequence generator's initial register state
  (`r_init`), an *l*-bit value (Table 6.1 below is for 16-bit `r_init`, i.e. `l = 16`, values
  shown MSB..LSB as `{r_(l-1), ..., r_0}`).

### Table 6.1 — Initial PN register state per `bootstrap_minor_version` (major version 0)
_§6.1.1, A/321:2026-06 p.14_

| `bootstrap_minor_version` | `r_init` (hex) |
|---|---|
| 0 | 0x019D |
| 1 | 0x00ED |
| 2 | 0x01E8 |
| 3 | 0x00E8 |
| 4 | 0x00FB |
| 5 | 0x0021 |
| 6 | 0x0054 |
| 7 | 0x00EC |

Note (informative): these seed values were chosen by minimizing cross-correlation among
candidate PN sequences relative to their own auto-correlation, per the spec's own note.

### Minor version 0 (`r_init = 0x019D`) signaling fields

When `r_init = 0x019D` (`bootstrap_minor_version = 0`), `N_S` shall equal 4. Bootstrap symbol 1
carries `N_b1 = 8` signaling bits (`b0_1 .. b7_1`, MSB to LSB), laid out as:

#### Table 6.2 — Signaling Fields for Bootstrap Symbol 1
_§6.1.1.1, A/321:2026-06 p.14-15_

| Syntax | No. of Bits | Format |
|---|---|---|
| `bootstrap_symbol_1() {` |
| `ea_wake_up_1` | 1 | uimsbf |
| `min_time_to_next` | 5 | uimsbf |
| `system_bandwidth` | 2 | uimsbf |
| `}` |

- **`ea_wake_up_1`** — bit 1 (LSB) of the 2-bit emergency-alert Wake-up Field (concatenated
  with `ea_wake_up_2` as the MSB). Bit semantics defined by A/321's normative reference [2],
  i.e. ATSC A/331 Annex G.2 (which is vendored and transcribed in this same directory — see
  [`a331-signalling.md`](a331-signalling.md) Annex G.2 summary below). See end of this
  document for the semantics extracted from that source.
- **`min_time_to_next`** — 5-bit index into a non-linear (piecewise-linear, increasing-step)
  time scale giving the minimum time interval, in ms, to the next frame matching the *same*
  major+minor version. Value 31 (`11111`) is reserved ("shall not be indicated"). The full
  28-value table:

#### Table 6.3 — Minimum Time Interval to Next Frame of the Same Major and Minor Version
_§6.1.1.1, A/321:2026-06 p.15_

| Index | Bit value | Minimum time interval (ms) |
|---|---|---|
| 0 | `00000` | 50 |
| 1 | `00001` | 100 |
| 2 | `00010` | 150 |
| 3 | `00011` | 200 |
| 4 | `00100` | 250 |
| 5 | `00101` | 300 |
| 6 | `00110` | 350 |
| 7 | `00111` | 400 |
| 8 | `01000` | 500 |
| 9 | `01001` | 600 |
| 10 | `01010` | 700 |
| 11 | `01011` | 800 |
| 12 | `01100` | 900 |
| 13 | `01101` | 1000 |
| 14 | `01110` | 1100 |
| 15 | `01111` | 1200 |
| 16 | `10000` | 1300 |
| 17 | `10001` | 1500 |
| 18 | `10010` | 1700 |
| 19 | `10011` | 1900 |
| 20 | `10100` | 2100 |
| 21 | `10101` | 2300 |
| 22 | `10110` | 2500 |
| 23 | `10111` | 2700 |
| 24 | `11000` | 2900 |
| 25 | `11001` | 3300 |
| 26 | `11010` | 3700 |
| 27 | `11011` | 4100 |
| 28 | `11100` | 4500 |
| 29 | `11101` | 4900 |
| 30 | `11110` | 5300 |
| 31 | `11111` | Not applicable (reserved) |

- **`system_bandwidth`** — 2-bit code for the post-bootstrap portion's system bandwidth:
  `00` = 6 MHz, `01` = 7 MHz, `10` = 8 MHz, `11` = "greater than 8 MHz" (reserved for future use
  — not expected to be signaled by receivers conforming to this version).

Bootstrap symbol 2 carries `N_b2 = 8` bits:

#### Table 6.4 — Signaling Fields for Bootstrap Symbol 2
_§6.1.1.1, A/321:2026-06 p.16_

| Syntax | No. of Bits | Format |
|---|---|---|
| `bootstrap_symbol_2() {` |
| `ea_wake_up_2` | 1 | uimsbf |
| `bsr_coefficient` | 7 | uimsbf |
| `}` |

- **`ea_wake_up_2`** — bit 2 (MSB) of the 2-bit emergency-alert Wake-up Field (see
  `ea_wake_up_1` above; defined by reference [2] = A/331 Annex G.2, transcribed below).
- **`bsr_coefficient`** — 7-bit unsigned value `N` (range 0-80 inclusive; 81-127 reserved) used
  as `Sample Rate Post-Bootstrap = (N + 16) x 0.384 MHz`.

Bootstrap symbol 3 carries `N_b3 = 8` bits:

#### Table 6.5 — Signaling Fields for Bootstrap Symbol 3
_§6.1.1.1, A/321:2026-06 p.16_

| Syntax | No. of Bits | Format |
|---|---|---|
| `bootstrap_symbol_3() {` |
| `preamble_structure` | 8 | uimsbf |
| `}` |

- **`preamble_structure`** — opaque 8-bit field whose *contents* are defined by whatever
  standard governs the post-bootstrap waveform (A/321 places no constraint on it itself); it
  signals the structure of the RF symbol(s) immediately following the bootstrap.

## 4. Bootstrap major version 1 (§6.2)

- ZC sequence root `q` = 197 when `bootstrap_major_version = 1`.
- Same `N_S >= 4` minimum.
- Minor version signaled the same way (PN register initial state), with its own seed table:

#### Table 6.6 — Initial PN register state per `bootstrap_minor_version` (major version 1)
_§6.2.1, A/321:2026-06 p.17_

| `bootstrap_minor_version` | `r_init` (hex) |
|---|---|
| 0 | 0xF110 |
| 1 | 0x3D21 |
| 2 | 0xE550 |
| 3 | 0xBD49 |
| 4 | 0x23CF |
| 5 | 0x0B50 |
| 6 | 0x3D3C |
| 7 | 0xA216 |

Minor version 0 under major version 1 (`r_init = 0xF110`) reuses the §6.1.1.1 field layout and
semantics (Tables 6.2/6.4/6.5 above) with these value-mapping differences (§6.2.1.1):

- `system_bandwidth` values may equivalently denote (per whatever document establishes the
  receiver capability set for this version) a *different* concrete bandwidth set — the spec's
  own worked example is `00 = 5.4 MHz, 01 = 6.3 MHz, 10 = 7.2 MHz, 11 = greater than 7.2 MHz`.
  This is explicitly not fixed by A/321 itself.
- `bsr_coefficient`: `Sample Rate Post-Bootstrap = (N + 10) x 0.384 MHz`, `N` in range 0-86
  inclusive (87-127 reserved) — a different additive constant and range than major version 0.
- `preamble_structure` semantics are, per the spec's own text, potentially interpretable per the
  standard defining the post-bootstrap signal — the spec gives illustrative (informative)
  examples (CAS bandwidth/cyclic-prefix/frequency-placement signaling; "preamble signal
  configuration defined in `[3]`" — an external ATSC reference not vendored here).

## 5. Future major versions (§6.3, normative)

ZC root values in the ranges **0..136, 138..196, and 198..1498 are Reserved** for future
`bootstrap_major_version` allocation (137 and 197 being the two versions defined by this
revision, for major versions 0 and 1 respectively).

## 6. Multiplexing multiple signal types via bootstrap prefixing (§7)

- Time-division multiplexing of different signal types in one RF channel is achieved by
  prefixing each physical layer frame (called an **Alternative Physical Layer Frame**, APLF, in
  this context) with its own bootstrap. Each APLF's bootstrap immediately follows the previous
  APLF's post-bootstrap waveform (within `max(T_A, T_B)` of the two frames' baseband sample
  periods, when they differ).
- APLFs are classified as **ATSC-native** (signal type specified by an ATSC document) or
  **externally-defined** (signal type specified by a non-ATSC organization).
- The **bootstrap major/minor version pair identifies the APLF's signal type** — this mapping
  is configured via a bootstrap-version allocation table (Tables 7.1/7.2 in A/321 are
  *illustrative examples* of such a table, not a fixed ATSC registry entry-by-entry; the
  concrete registry is presumably the ATSC Code Point Registry referenced elsewhere in the
  ATSC 3.0 suite, not reproduced in A/321 itself). Per §7.3: bootstrap version 0.0 identifies
  ATSC 3.0 itself (as specified in A/321's reference `[3]`, i.e. A/322).

Note (informative, not transcribed): Annex A of A/321 ("Example Method of Gray Code De-mapping
at Receiver") and the underlying OFDM/ZC/PN signal-generation math in §5.2-§5.4 (frequency
domain sequence generation, IFFT, cyclic shift structure) are physical-layer DSP procedures, not
wire-format syntax a `no_std` byte-oriented parser crate would implement; they are out of scope
for this document and not transcribed here. See [`README.md`](README.md) for the full list of
things this pass did not establish.

## 7. Reference [2] — `ea_wake_up` semantics (A/331 Annex G.2)

A/321's normative reference [2] is **ATSC A/331, "Signaling, Delivery, Synchronization, and
Error Protection"** (A/321:2026-06 cites A/331:2026-04). The `ea_wake_up_1` / `ea_wake_up_2`
bit semantics are defined in A/331 Annex G.2, transcribed here from the source document (both
A/331:2025-06, which was initially transcribed for this PR, and A/331:2026-04, verified as
having identical Annex G.2 content):

- The two bits are concatenated into a 2-bit **Wake-up Field**: `{ea_wake_up_2, ea_wake_up_1}`,
  i.e. `ea_wake_up_1` is the LSB, `ea_wake_up_2` is the MSB.
- When an AEA message with `AEA@wakeup="true"` is present, Wake-up Field shall be non-zero.
- Wake-up Field shall change when an AEA message with `AEA@wakeup="true"` is added.
- Wake-up Field *may* change when an AEA message with `AEA@wakeup="true"` is changed.
- When no AEA messages have `AEA@wakeup="true"`, Wake-up Field shall be `00`.

### Table G.2.1 — Meaning of Wake-up Field

| Value | Meaning |
|---|---|
| `00` | No emergency to wake up devices is currently signaled |
| `01` | Emergency to wake up devices — setting 1 |
| `10` | Emergency to wake up devices — setting 2 |
| `11` | Emergency to wake up devices — setting 3 |

Note (informative): the Wake-up Field does not encode a one-to-one correspondence to a
particular alert; canceling the latest alert does not revert the field value while another alert
that triggered a field change remains active. A receiver that powers off while the field is
non-zero should remain off unless a *new* non-zero value is transmitted. A/331 Annex G.2 places
this in the context of the AEA (Advanced Emergency Alerting) message framework, described
further in A/331 §6.5 (not transcribed in this pass — see
[`a331-signalling.md`](a331-signalling.md)).
