# Changelog

## [Unreleased]

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
## [9.2.0] - 2026-08-02

### Added
- **`cenc::CencScheme`** — the Common Encryption scheme identity
  (`schm.scheme_type`, ISO/IEC 23001-7 §4: `cenc` / `cbcs`), with `name()`,
  `to_four_cc()`, `from_four_cc()`, `Display`, and the `SCHEME_CENC` /
  `SCHEME_CBCS` four-CC constants. Re-exported at the crate root as
  `broadcast_common::CencScheme`.

  It moved here from `transmux::cenc` (issue #878) because two sibling crates
  now need it and cannot share it any other way: `transmux` (which owns the
  ISOBMFF `schm`/`tenc`/`senc` boxes that carry it) and `broadcast-hls` (which
  renders the HLS `#EXT-X-KEY` tag from it). CENC is *Common* Encryption — a
  container-independent identity — so it belongs below both, alongside the
  `Encrypt`/`Decrypt` traits it parameterises. Issue #564 had already
  consolidated three copies into one; this preserves that across the crate
  split instead of reintroducing a second definition.
- **`hex::hex_encode`** — lowercase hex encoding of a byte slice, shared by
  every wire format here that renders an opaque byte field as text (HLS
  `KEYID=0x…`, SDP `fmtp`, Smooth `CodecPrivateData`). Moved from
  `transmux::rtp`, which now re-exports it. Only the *encoder* is shared;
  decoders stay with their callers, each reporting through its own error type.
- **`serde` feature** (off by default) — derives `serde::Serialize` on the
  shared spec types (currently `CencScheme`). Pulls no dependency unless
  enabled.

Both additions are purely additive — no existing item changed — so every
`broadcast-common = "9"` consumer is unaffected. Crates using the new items
floor at `9.2`.

## [9.1.0] - 2026-07-30

Lockstep release with dvb-conformance 9.1.0; no functional changes.

### Added
- `tests/label_coverage.rs` drift guard (issue #806).
- `tests/workspace_drift_guard_coverage.rs`: a workspace-level guard asserting
  every library crate carries both the `label_coverage.rs` and
  `non_exhaustive_coverage.rs` drift guards (or a recorded exemption), so a
  newly added crate cannot silently skip either convention (issue #806).
  No public API or behaviour change.

## [9.0.0] - 2026-07-27
### Changed (Breaking)
- **`Encrypt::encrypt` now takes `&mut self`, not `&self`.** Needed so a
  stateful implementor (e.g. `transmux`'s `CencEncryptor`) can own a running
  per-key IV counter and advance it across calls instead of restarting it
  every time — the fix for a duplicate-IV/two-time-pad defect (see
  `transmux` 0.20.0's CHANGELOG for the full incident). **Any external
  `impl Encrypt` must change its `encrypt` method signature from
  `fn encrypt(&self, …)` to `fn encrypt(&mut self, …)`; any caller holding
  `&dyn Encrypt` (or generic `E: Encrypt`) must obtain/hold a mutable
  reference/binding instead of a shared one.** This is why the lockstep six
  move to 9.0.0 rather than the 8.7.0 this change first landed under
  (never published — crates.io was still at 8.6.0 when this was corrected).
  Retitled from the unpublished `[8.7.0]` entry below; its `Added` content
  is preserved as-is.

### Added
- `stage` module: the `Stage` incremental-drive trait (`feed`/`poll`/`finish`/
  `next_deadline`/`on_deadline`/`demand`) unifying the drive shape of every
  streaming stage in the workspace, plus its supporting types —
  `Timestamp` (a `no_std`-safe monotonic-nanosecond newtype standing in for
  `std::time::Instant`, with saturating arithmetic and a `std`-only
  `from_instant` convenience), `Demand` (an advisory backpressure hint), and
  `type In<'a>` (the associated input type, added in step 2 to support borrowed
  packet arrays without lifetime explosion across demux variants).
  Media-plane migration step 1 (additive, zero cascade to the ~30 workspace
  dependents pinned on `version = "8"`).

### Changed
- `mux` module docs: scope `Package`/`Unpackage`/`Encrypt`/`Decrypt` explicitly
  as the *batch / whole-file* container-mux contract, distinct from `stage`'s
  new *incremental* contract. Documentation only, no behaviour change.

## [8.5.0] - 2026-07-21
### Changed
- Lockstep minor release riding the `mpeg-ts` 0.3.0 compatibility cycle
  (issue #663). No functional or API change to `broadcast-common`.

## [8.4.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [8.3.0] - 2026-07-03
### Changed
- Lockstep release with the DVB core crates; no functional changes to `broadcast-common`.

## [8.2.1] — 2026-07-02

### Documentation
- Document the `mux` container-mux traits (`Unpackage`/`Package`/`Encrypt`/`Decrypt`)
  in the crate-root docs and README. The 8.2.0 release shipped the `mux` module
  but its published docs did not describe it; this fixes that on the next release.

## [8.2.0] — 2026-07-02

### Added
- `mux` module with four abstract container-mux traits: `Unpackage` / `Package`
  (demux ⇄ mux between a packaged container and an in-memory media IR) and
  `Decrypt` / `Encrypt` (in-place sample (un)protection). Each is generic over
  its input/output/media/config/key associated types plus `type Error`,
  mirroring the `Parse` / `Serialize` style; no concrete media or codec types
  appear here. Re-exported at the crate root.

## [8.1.0] — 2026-06-29

### Changed
- Renamed from `dvb-common` (same API, same content). `dvb-common` continues as a deprecated re-export shim.

## [8.0.0] — 2026-06-27

### Changed
- Lockstep major release; parity bump. No functional changes to this crate.

## [7.9.0] — 2026-06-22

### Changed

- Lockstep release; no functional changes to this crate.

## [7.8.0] — 2026-06-21

### Changed

- Lockstep release; no functional changes to this crate.

## [7.7.1] — 2026-06-21

### Changed
- Lockstep release; no functional changes to this crate.

## [7.7.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.6.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.5.0] — 2026-06-19

### Added
- `examples/`: `crc_and_bcd` (CRC-32/MPEG-2 + BCD coding) and
  `implement_parse_serialize` (implementing the symmetric `Parse`/`Serialize`
  contract for your own wire type).

## [7.4.0] — 2026-06-18

Lockstep release; no functional changes.

## [7.3.0] — 2026-06-17

### Changed
- Lockstep release; no functional changes to this crate.

## [7.2.0] — 2026-06-16

### Changed
- Lockstep release; no functional changes to this crate.

## [7.1.0] — 2026-06-15

### Changed
- Internal: byte-readers hardened with bounds-checked `split_first_chunk` /
  `split_last_chunk` / `first_chunk`, folding manual length checks into the read.
  No API or behaviour change (#207).

## [7.0.0] — 2026-06-14

**BREAKING (MSRV 1.75 → 1.81).**

### Added
- **no_std + alloc support** (#63). `#![no_std]` when the new default `std`
  feature is off; only `alloc` is required. `BitError` now impls
  `core::error::Error`. The MJD↔calendar codec uses `libm::floor` (bit-identical
  to `f64::floor`, which is std-only) so it is no_std. chrono is configured
  no_std+alloc (clock/std gated behind `std`).

### Changed
- MSRV is now **1.81** (for `core::error::Error`).

## [6.7.0] — 2026-06-14

### Added
- `impl_spec_display!` — exported declarative macro that generates a `Display`
  impl for a spec/field enum, delegating to its inherent `name()` and preserving
  the byte on `Reserved(u8)`-style arms as `"{name}(0x{:02X})"` (#204).

## [6.6.0] — 2026-06-14

### Added
- **`bits` module** — `BitReader` / `BitWriter`, an MSB-first (DVB/MPEG bit
  order) reader/writer for dense sub-byte wire fields. Reads/writes 0..=64 bits
  per call with `BufferTooShort`-style `BitError` context; the writer sets *and*
  clears each target bit (no pre-zeroing needed). Foundational for the
  EN 302 755 §7.2 L1 signalling parser (#54).
- Criterion benchmark suite (`benches/crc32.rs`) measuring `crc32_mpeg2::compute`
  throughput at 188 B / 4096 B / 65536 B payload sizes — dev-only, no API change (#62).

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

All notable changes to the `broadcast_common` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] — 2026-06-05

Lockstep release with the `dvb-si` 3.x line. The workspace-wide move to
Serialize-only serde does not affect this crate — its `Parse` / `Serialize`
traits are first-party (not serde) and it never carried any serde `Deserialize`.
The `yoke` feature added downstream needs nothing here: `broadcast-common` exposes no
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
