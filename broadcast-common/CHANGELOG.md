# Changelog

## [Unreleased]

## [10.0.0] - 2026-10-17
### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Fixed (BREAKING-class behaviour, adversarial review of #1268)
- `time`: the MJD to calendar conversion (`decode_mjd_bcd`, `encode_mjd_bcd`, `mjd_to_ymd`, `ymd_to_mjd`, the chrono wrappers) is now integer proleptic-Gregorian arithmetic valid over the whole v1.20.1 range (MJD 32768 ..= 98303, 1948-08-05 ..= 2128-01-09). It still used the EN 300 468 Annex C closed-form formula, which that Annex's NOTE 2 limits to 1900-03-01 ..= 2100-02-28: wire values `0x5840 ..= 0x7FFF` decoded one day early (`58 40` gave the non-existent 2100-02-29 instead of 2100-03-01, `7F FF` gave 2128-01-08), and the encoder accepted 2100-02-29. Tests walk every 16-bit wire value in MJD order (consecutive-day check + byte-exact round trip) and pin 2000-02-29, 2038-04-22/23, 2100-02-28/03-01 and 2128-01-09. The `libm` dependency is dropped (no `floor` any more).

### Changed (BREAKING, #1222)
- `time`: the DVB 16-bit MJD field follows the EN 300 468 v1.20.1 2038 wrap rule (msb 0 => MJD = 0x10000 + field). `decode_mjd_bcd`/`mjd_to_ymd`/`decode_mjd_bcd_utc` and their encoders now cover 1948-08-05 ..= 2128-01-09; earlier dates no longer encode/decode.

### Added
- `len::fit_mask(value, mask, field)`: checked replacement for the `(value & MASK)` idiom in serializers; returns `FieldOverflow` when a set bit lies outside `mask` (#1197).
- `clock33`: the 27 MHz PCR clock in one place - `PTS_HZ`, `PCR_HZ`, `PCR_EXT_MODULUS`, `PCR_WRAP_27MHZ` (`2^33 * 300`), `pcr_from_parts` / `pcr_split` (`base * 300 + extension`), `pcr_forward_distance` and `pcr_signed_distance` (the modular PCR delta; `wrapping_sub(..) % PCR_WRAP_27MHZ` is wrong across a wrap because the modulus is not a power of two). Replaces five hand-rolled copies in `dvb-conformance`, `media-doctor`, `compliance-probe`, `dvb-tools`, `ts-fix` and the `base * 300 + ext` arithmetic of `mpeg-ts` `Pcr`, `mpeg-pes` `Escr`, `mpeg-ps` `Scr` (#1206).
- `clock33::{Marked, MARKERS_ALL_SET, unpack_marked, pack_marked, pack_marked_with}`: the 5-byte marker-bit timestamp field (`prefix(4) ts[32:30](3) 1 ts[29:15](15) 1 ts[14:0](15) 1` - PES PTS/DTS, the seamless-splice `DTS_next_AU`, ST 2038 PTS) in one place, decoded without failing so each caller keeps its own prefix/marker validation and error type. Pinned by a REAL capture PTS field (`29 FD 91 D2 29` = 5 358 512 404, decoded independently) plus every boundary / marker combination (#1208).
- `parity` module: `has_odd_parity`, `set_odd_parity`, `strip_parity` - the odd-parity protected byte shared by CEA-608 line-21 data and EBU Teletext `txt_data_block` bytes (ETSI EN 300 706 section 8.1), detect-only by design. Replaces four hand-rolled copies in `timed-metadata`, `cc-data` and `caption-convert` (#1212).

### Removed (BREAKING)
- `Serialize::to_bytes()`, the default method that panicked when `serialize_into` failed, is removed (umbrella #1143 / T15). `Serialize::try_to_bytes() -> Result<Vec<u8>, Self::Error>` is the only allocate-and-serialize entry point; production code propagates the error, tests use `.try_to_bytes().unwrap()`. No infallible replacement is provided (no `Infallible` marker): a panic-free default cannot be offered for a trait whose implementers may fail. Guarded by `tests/no_panicking_to_bytes_guard.rs`. Inherent `to_bytes` methods on concrete types (fixed-size arrays, `Result`-returning ones) are unaffected. Major bump of `broadcast-common`; every dependent crate's `broadcast-common` caret moves with it.

### Fixed
- `time::encode_mjd_bcd` rejects calendar-impossible days (31 Feb, 31 Apr, 29 Feb in a non-leap year): they used to encode silently as a later day that decode would not return (audit r01-W16, #1073).
### Added
- `Timestamp::saturating_add_nanos`; `checked_add_nanos` (which always saturated, never checked) is deprecated in its favour (audit r01-O8).

### Changed
- Docs only: crate-level "Reserved-bit and trailing-byte policy" section stating the workspace rule (tolerant parse, spec-value serialize, preserve-on-parse exceptions, explicit reserved writes) (#1139).
- Docs only: `Serialize::to_bytes` no longer claims it can fail only for hand-constructed values; it can also fail for values a lenient decoder assembled from damaged wire data (transmux #1171). Use `try_to_bytes` for anything derived from untrusted input.

## [9.4.0] - 2026-10-05
### Added
- **`seq`** — `SeqSpace`: wrap-safe modular sequence-number arithmetic (`add`/`diff`/`lt`/`leq`/`gt`/`geq`/`in_closed_range`) over a `bits`-wide space, the one algorithm behind `rist-runtime` (16-bit) and `srt-runtime` (31-bit) `arq::seq` (audit r08-RIST-O1, #1141).
- **`pusi::PusiAccumulator`** (+ `DEFAULT_MAX_UNIT_SIZE`, 16 MiB) — the one PUSI-delimited unit-accumulation rule (ISO/IEC 13818-1 §2.4.3.2: ignore payload before the first PUSI, a PUSI closes the in-progress unit, a capped unit is discarded until the next PUSI), shared by `mpeg-pes::PesAssembler` and `mpeg-ts::PusiReassembler` instead of two diverged copies (audit r01-W13, #1074).
- **`len`** — `FieldOverflow` + the `fit_bits`/`fit_u8`/`fit_u16`/`fit_u24`/
  `fit_u32` helpers, for serializers to reject, not truncate, oversized
  length/count fields: the comparison happens before any narrowing, so a
  value an `as u16` would silently wrap (e.g. 65 540 → 4) is returned as a
  `FieldOverflow { field, value, max }` the caller maps into its own error
  type instead of emitting a corrupt frame; see #1129.
- **`Serialize::try_to_bytes`** — allocates and serializes like `to_bytes`
  but returns the serializer's error instead of panicking, truncating to the
  byte count actually written; prefer it for hand-built values that can
  violate a wire constraint (#1143).
- **`clock33::{add, add_signed, signed_distance}`** — modular arithmetic on
  the 33-bit PTS/DTS clock: `(a + b) mod 2^33` (e.g. SCTE-35 `pts_time` +
  `pts_adjustment`), a signed-delta variant always landing in `[0, 2^33)`,
  and the shortest signed distance between two samples in `(-2^32, 2^32]`;
  inputs at or above the modulus are reduced first (#1137).

### Fixed
- **`time::encode_mjd_bcd`** — now rejects `hour > 23` / `minute > 59` /
  `second > 59` (matching the ranges `decode_mjd_bcd` enforces); previously
  it only bottomed out at `to_bcd_byte`'s `<= 99` check, so it could encode
  a value `decode_mjd_bcd` then refused to decode back (#1073).
- **`time`** — the MJD↔calendar conversion algorithm (EN 300 468 Annex C)
  existed as two independent hand-copies (the dependency-free `_nogate`
  helpers and the `chrono`-gated public functions); both now share one
  `mjd_to_ymd_core`/`ymd_to_mjd_core` implementation so the two can no
  longer silently drift apart (#1073).
- **`ts_dup::is_legal_duplicate_pair`** — the PCR exemption now requires
  `adaptation_field_length >= 7` (flags byte + 6-byte PCR), not just that
  the packet buffer is long enough; a packet whose adaptation field claims
  `PCR_flag` but declares too little length no longer has trailing payload
  bytes wrongly treated as the exempt PCR field (#1073).

### Changed
- `hex::hex_encode` now delegates to the `hex` crate (`no_std` + `alloc`); output and signature unchanged. No API change.

## [9.3.0] - 2026-08-11

### Added
- **`ts_dup`** — the ITU-T H.222.0 (08/2023) / ISO/IEC 13818-1 §2.4.3.3
  "legal duplicate transport-stream-packet" check: `is_legal_duplicate_pair`
  (byte-for-byte identity between two raw TS packets, with the 6-byte PCR
  field the sole exception, and `adaptation_field_control` required to be
  `01`/`11`) and `check_duplicate` (folds in the spec's "two, and only two
  consecutive" cardinality rule via a caller-tracked `dup_already_used`
  flag, returning a `DuplicateVerdict`). Consolidates three independent,
  disagreeing hand-rolled copies of this rule: `dvb-conformance`'s
  `canonical_packet_for_dup_check` (fixed for property 1 only in #956),
  `media-doctor`'s `cc_anomaly` (payload-only comparison — too lenient on
  property 1, and never enforced property 3 at all), and `ts-fix`'s test
  and production duplicate-detection hashes. `no_std`, no allocation.
- **`clock33`** — generic 33-bit wrapping-clock helpers: `unwrap_delta`
  (bidirectional wrap-corrected unroll of a repeating 90 kHz counter into an
  ever-growing accumulator) and `wrapping_forward_distance` (stateless
  modular forward-distance for a wrap-vs-past comparison), plus the
  `WRAP_33BIT`/`WRAP_33BIT_HALF` constants. Consolidates four independent
  hand-rolled copies of this math (`timed-metadata::Timeline`,
  `transmux::ts_demux`, `media-doctor::pts_check`,
  `compliance-probe::scte35`) into one owner — a duplication-audit finding.
  `transmux` cannot depend on `timed-metadata` (a signalling-conversion
  crate several layers up the stack) without an inverted, heavy edge, so the
  primitive lives here instead, in the crate every one of the four already
  depends on. Also fixes a latent correctness gap the consolidation
  surfaced: `timed-metadata`'s previous forward-only epoch counter
  misclassified a small backward reorder that straddles the wrap origin as
  a huge forward jump; `unwrap_delta`'s bidirectional correction (already
  used by `transmux`) gets this right.

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
