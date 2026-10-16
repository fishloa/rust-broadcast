---
status: DRAFT
---
# broadcast-common 10.0.0

_Released 2026-10-16._

**Breaking release.** The foundation crate takes its tenth major: the panicking `Serialize::to_bytes()` default method is gone (`try_to_bytes()` is the only allocate-and-serialize entry point), and the DVB 16-bit MJD field follows the EN 300 468 v1.20.1 2038 wrap rule. It also grows the shared building blocks the wave moved out of ten crates: `clock33` PCR/marker-bit helpers, `parity`, and `len::fit_mask`. Because 49 workspace crates depend on it, **its caret epoch moving 9 -> 10 forces a major-class release of every dependent** (see the wave overview).

## Why this version

- **Proposed version:** 9.4.0 -> **10.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (Serialize::to_bytes removed; MJD range changed (CHANGELOG BREAKING, semver-checks major)).
- **cargo-semver-checks vs live 9.4.0 (all features):** MAJOR (1 major, 1 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** MJD range changed (CHANGELOG BREAKING, semver-checks major). Siblings it depends on that move epoch in this wave: none.
- **Caret epoch:** 9 -> 10; every workspace dependent must move with it.

## Dependency changes

No workspace sibling requirement changes.

## Breaking changes and migration

### 1. `Serialize::to_bytes()` is removed (umbrella #1143 / audit T15)

The default method panicked when `serialize_into` failed. `Serialize::try_to_bytes() -> Result<Vec<u8>, Self::Error>` is the only allocate-and-serialize entry point. No infallible replacement is offered: a panic-free default cannot exist for a trait whose implementers may fail. Inherent `to_bytes` methods on concrete types (fixed-size arrays, `Result`-returning ones) are unaffected. Guarded by `tests/no_panicking_to_bytes_guard.rs`.

```rust
// before
let wire: Vec<u8> = section.to_bytes();
// after: production code propagates the error
let wire: Vec<u8> = section.try_to_bytes()?;
// after: tests
let wire = section.try_to_bytes().unwrap();
```

### 2. The DVB 16-bit MJD follows the 2038 wrap rule (#1222, EN 300 468 v1.20.1)

`time::decode_mjd_bcd`, `mjd_to_ymd`, `decode_mjd_bcd_utc` and their encoders now cover **1948-08-05 ..= 2128-01-09**: when the msb of the 16-bit field is 0 the MJD is `0x10000 + field`. Dates before 1948-08-05 no longer encode or decode; `0x03D1123456` is 2040-12-25 12:34:56. A caller storing pre-1948 dates (archive metadata) must handle them outside this codec.

`time::encode_mjd_bcd` additionally rejects calendar-impossible days (31 Feb, 31 Apr, 29 Feb in a non-leap year) that used to encode silently as a later day (#1073).

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed (BREAKING, #1222)
- `time`: the DVB 16-bit MJD field follows the EN 300 468 v1.20.1 2038 wrap rule (msb 0 => MJD = 0x10000 + field). `decode_mjd_bcd`/`mjd_to_ymd`/`decode_mjd_bcd_utc` and their encoders now cover 1948-08-05 ..= 2128-01-09; earlier dates no longer encode/decode.

#### Added
- `len::fit_mask(value, mask, field)`: checked replacement for the `(value & MASK)` idiom in serializers; returns `FieldOverflow` when a set bit lies outside `mask` (#1197).
- `clock33`: the 27 MHz PCR clock in one place - `PTS_HZ`, `PCR_HZ`, `PCR_EXT_MODULUS`, `PCR_WRAP_27MHZ` (`2^33 * 300`), `pcr_from_parts` / `pcr_split` (`base * 300 + extension`), `pcr_forward_distance` and `pcr_signed_distance` (the modular PCR delta; `wrapping_sub(..) % PCR_WRAP_27MHZ` is wrong across a wrap because the modulus is not a power of two). Replaces five hand-rolled copies in `dvb-conformance`, `media-doctor`, `compliance-probe`, `dvb-tools`, `ts-fix` and the `base * 300 + ext` arithmetic of `mpeg-ts` `Pcr`, `mpeg-pes` `Escr`, `mpeg-ps` `Scr` (#1206).
- `clock33::{Marked, MARKERS_ALL_SET, unpack_marked, pack_marked, pack_marked_with}`: the 5-byte marker-bit timestamp field (`prefix(4) ts[32:30](3) 1 ts[29:15](15) 1 ts[14:0](15) 1` - PES PTS/DTS, the seamless-splice `DTS_next_AU`, ST 2038 PTS) in one place, decoded without failing so each caller keeps its own prefix/marker validation and error type. Pinned by a REAL capture PTS field (`29 FD 91 D2 29` = 5 358 512 404, decoded independently) plus every boundary / marker combination (#1208).
- `parity` module: `has_odd_parity`, `set_odd_parity`, `strip_parity` - the odd-parity protected byte shared by CEA-608 line-21 data and EBU Teletext `txt_data_block` bytes (ETSI EN 300 706 section 8.1), detect-only by design. Replaces four hand-rolled copies in `timed-metadata`, `cc-data` and `caption-convert` (#1212).

#### Removed (BREAKING)
- `Serialize::to_bytes()`, the default method that panicked when `serialize_into` failed, is removed (umbrella #1143 / T15). `Serialize::try_to_bytes() -> Result<Vec<u8>, Self::Error>` is the only allocate-and-serialize entry point; production code propagates the error, tests use `.try_to_bytes().unwrap()`. No infallible replacement is provided (no `Infallible` marker): a panic-free default cannot be offered for a trait whose implementers may fail. Guarded by `tests/no_panicking_to_bytes_guard.rs`. Inherent `to_bytes` methods on concrete types (fixed-size arrays, `Result`-returning ones) are unaffected. Major bump of `broadcast-common`; every dependent crate's `broadcast-common` caret moves with it.

#### Fixed
- `time::encode_mjd_bcd` rejects calendar-impossible days (31 Feb, 31 Apr, 29 Feb in a non-leap year): they used to encode silently as a later day that decode would not return (audit r01-W16, #1073).
#### Added
- `Timestamp::saturating_add_nanos`; `checked_add_nanos` (which always saturated, never checked) is deprecated in its favour (audit r01-O8).

#### Changed
- Docs only: crate-level "Reserved-bit and trailing-byte policy" section stating the workspace rule (tolerant parse, spec-value serialize, preserve-on-parse exceptions, explicit reserved writes) (#1139).
- Docs only: `Serialize::to_bytes` no longer claims it can fail only for hand-constructed values; it can also fail for values a lenient decoder assembled from damaged wire data (transmux #1171). Use `try_to_bytes` for anything derived from untrusted input.

## Merged changes since `broadcast-common-v9.4.0`

- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- feat(dvb-si): EN 300 468 v1.20.1 (2038 MJD wrap, SAT centre/occurrence, new component types) (#1223)
- broadcast_common::parity: shared odd-parity helpers; migrate cc-data, timed-metadata, caption-convert (#1212) (#1213)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: pack_marked/unpack_marked for the 5-byte marker-bit timestamp; migrate mpeg-pes, mpeg-ts, st291, transmux (#1208) [stacked on #1207] (#1209)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- dvb-si: checked fit_mask for serializer masks, SAT reserved runs, stuffing SSI, reserved-default evidence (#1197) (#1199)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
- fix(transmux): ignore invalid H.264/HEVC parameter sets; no unserializable config (#1171), release 0.25.6 (#1172)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
