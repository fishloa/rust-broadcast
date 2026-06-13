# Changelog

## [Unreleased]

## [6.5.0] — 2026-06-13

### Added
- **`time::SECS_2000_EPOCH`** — Unix timestamp (946 684 800) of the T2-MI / DVB
  time-of-day epoch 2000-01-01T00:00:00 UTC (#47).
- **`time::decode_seconds_since_2000_utc()`** (feature `chrono`) — decodes a
  `seconds_since_2000` + subsecond-nanos pair + `utco` offset to a `DateTime<Utc>`.
- **`time::encode_seconds_since_2000_utc()`** (feature `chrono`) — inverse encoder.

## [6.4.0] — 2026-06-13

Version-lockstep release with the workspace (#158 spec-table drift-guards + spec-fidelity audit; dvb-si PMT section/last-section fields; dvb-bbframe DVB-S2 BUFSTAT ISSY decode). No changes to this crate.

## [6.3.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-scte35` crate; dvb-si `TsResync` byte-stream resync helper). No changes to this crate.

## [6.2.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-tools` and
`dvb-conformance` crates; dvb-t2mi per-PLP inner-TS filter). No changes to this
crate.

## [6.1.0] — 2026-06-12

Version-lockstep release with the workspace (dvb-si SI output half + correctness
pass + pre-release audit; dvb-bbframe chain conveniences). No changes to this crate.

## [6.0.0] — 2026-06-11

Lockstep major with the workspace (the dvb-si/dvb-t2mi "decode completeness"
release — typed enums for every spec-enumerated wire code). No changes to this
crate.

## [5.0.0] — 2026-06-11

Lockstep major across the workspace (the `dvb-si` 5.0 "type everything +
harden" release). One small breaking change to this crate.

### Changed
- **`crc32_mpeg2::TABLE` is now `pub(crate)`** (was `pub`). The 256-entry
  lookup table is an internal implementation detail; only `compute()` and
  `POLY` remain public. (Breaking only if you referenced the table directly.)

## [4.3.0] — 2026-06-10

Version-lockstep release with the workspace (dvb-si epg / resync /
adaptation-field+PCR, dvb-t2mi decoded timestamps, dvb-bbframe buffer-reusing
extractor); no changes to this crate.

## [4.2.0] — 2026-06-09

Version-lockstep release with the workspace (dvb-si DSM-CC `ModuleReassembler`
hardening, #42 / #43); no changes to this crate.

## [4.1.0] — 2026-06-09

### Added
- `bcd` module — binary-coded-decimal codec (`from_bcd_byte` / `to_bcd_byte`,
  `bcd_to_decimal` / `decimal_to_bcd`), dependency-free.
- `time` module — BCD `HHMMSS` duration codec (`decode_bcd_duration` /
  `encode_bcd_duration`, dependency-free) plus a MJD↔calendar UTC codec
  (`mjd_to_ymd` / `ymd_to_mjd`, `decode_mjd_bcd_utc` / `encode_mjd_bcd_utc`;
  EN 300 468 Annex C) behind a new optional **`chrono`** feature. The default
  build stays dependency-free. These de-dup the MJD/BCD logic previously copied
  in `dvb-si`.

## [4.0.0] — 2026-06-08

Version-lockstep release with the workspace (the `dvb-si` 4.0 section/table
split — `*Section` parsers and the new `collect` module); no changes to this
crate.

## [3.1.2] — 2026-06-07

Version-lockstep release with the workspace (dvb-si spanning-into-PUSI section fix); no changes to this crate.

## [3.1.1] — 2026-06-07

Version-lockstep release with the workspace (dvb-si `SectionReassembler`
concatenated-section fix); no changes to this crate.

All notable changes to the `dvb_common` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] — 2026-06-05

Lockstep release with the `dvb-si` 3.x line. The workspace-wide move to
Serialize-only serde does not affect this crate — its `Parse` / `Serialize`
traits are first-party (not serde) and it never carried any serde `Deserialize`.
The `yoke` feature added downstream needs nothing here: `dvb-common` exposes no
borrowing view types (only the `Parse<'a>` trait). No functional changes.

## [2.1.0] — 2026-06-05

Version-lockstep release with the workspace (Any* `name()` additions in
dvb-si / dvb-t2mi); no functional changes.

## [2.0.0] — 2026-06-05

Version-lockstep release with the workspace (dvb-si 2.0 typed client API);
no functional changes.

## [1.1.0] — 2026-06-04

Version-lockstep release with the workspace (dvb-si: complete EN 300 468
Table 12 descriptor coverage); no functional changes.

## [1.0.1] — 2026-06-04

Version-lockstep release with the workspace (dvb-si README overhaul);
no code changes.

## [1.0.0] — 2026-06-04

Initial release. Shared `Parse<'a>` / `Serialize` traits and `crc32_mpeg2`
(CRC-32 per ETSI EN 300 468 Annex C / ETSI TS 102 773 Annex A) used by the
`dvb_si`, `dvb_t2mi`, and `dvb_bbframe` family. Zero runtime dependencies.
