# Changelog

## [Unreleased]

### Added
- Criterion benchmark suite (`benches/bbframe_hot_paths.rs`) measuring
  `BbframePump::feed` on the TNT real-capture fixture and `up_iter`/`NmTsIter`
  iterator throughput on a synthetic NM data field — dev-only, no API change (#62).

## [6.5.0] — 2026-06-13

Version-lockstep release with the workspace (#47 T2 emission-time accessors; #50 SSU GroupInfoIndication + data_broadcast_id 0x000A selector; #53 S2Xv2 0x24 extension descriptor). No changes to this crate.

## [6.4.0] — 2026-06-13

### Added
- `SignallingKind::BufStat { bufstat, units }` — decodes the DVB-S2 **BUFSTAT**
  ISSY signalling type (`[5:4]=0b10`, the `0xEXXXXX` code range) per EN 302 307-1
  Annex D Table D.1, with `bufstat_bits()` / `bufstat_bytes()` accessors.
  Previously this code range decoded to `SignallingKind::Reserved` (#182).
- `docs/en_302_755_t2.md` Annex C — Table C.1 (ISSY field coding) transcribed
  verbatim from the PDF; the `issy.rs` ISCR/BUFS/TTO decode was verified
  cell-by-cell against it (#183).
- New `tests/spec_drift.rs` with drift-guard coverage for two code-backing enums
  (#158): `TsGs` (EN 302 755 Table 1, 4 variants) and `BufsUnit` (Annex C Table
  C.1, 4 variants). Adds `spec_tables/ts_gs.toml` and `spec_tables/bufs_unit.toml`
  data mirrors. Test/data only.

### Fixed
- `Bbheader::serialize_into` in HEM with `issy_in_header = None` left stale bytes
  at positions `buf[2]`, `buf[3]`, and `buf[6]` when the caller's buffer was not
  zero-initialised, corrupting the CRC-8.  Those positions are now explicitly
  written to zero, making `serialize_into` fully deterministic regardless of
  incoming buffer content and matching the output of `to_bytes()`.
- `CarryOverExtractor::feed_hem_into` / `feed_nm_into` mishandled `SYNCD = 65535`
  (0xFFFF) — the spec-defined sentinel meaning "no UP starts in the DATA FIELD"
  (EN 302 755 Table 2).  The old code computed `syncd_bytes = 65535 / 8 = 8191`,
  which never matched the actual `need`, so the pending partial was discarded
  (`partial_discards++`) instead of being continued.  `SYNCD = 0xFFFF` is now
  handled as a distinct case before the stride-mismatch logic in both the HEM and
  NM paths: the entire data field is appended to the pending partial; if the
  partial thereby reaches its full length it is emitted, otherwise it continues to
  the next frame.

## [6.3.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-scte35` crate; dvb-si `TsResync` byte-stream resync helper). No changes to this crate.

## [6.2.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-tools` and
`dvb-conformance` crates; dvb-t2mi per-PLP inner-TS filter). No changes to this
crate.

## [6.1.0] — 2026-06-12

Lockstep minor with the workspace; #55 chain conveniences for this crate.

### Added
- Decoded ISSY accessors `BufsUnit::multiplier_bits`,
  `SignallingKind::bufs_bits` / `bufs_bytes` / `tto_t_over_256`
  (#55, EN 302 755 Annex C).
- `BbframePump` + `BbframePumpStats` (#55) — per-PLP BBFrame→inner-TS pump that
  orchestrates BBHEADER parse, mode detection, and per-PLP carry-over extraction
  in one infallible `feed(plp_id, df_bytes) -> &[[u8; 188]]` call. Composes
  `CarryOverExtractor`; no `dvb-t2mi` dependency. (DVB-S2 NM/HEM user-packet
  framing, EN 302 307-1 §5.1.4 / EN 302 755 Annex F.)
- GSE handoff doc example (#55c) — crate-root section showing the `TsGs::Gse`
  match arm and how to pass the data field to the `dvb-gse` crate for GSE packet
  parsing.

## [6.0.0] — 2026-06-11

Lockstep major with the workspace decode-completeness release; additive decode
accessors here.

### Added
- `Matype::roll_off()` — decodes the MATYPE `ext` roll-off bits `[1:0]`
  (`RollOff`: α 0.35 / 0.25 / 0.20, reserved) in the DVB-S2 context.
- `Issy::Signalling` BUFS/TTO sub-coding accessor (EN 302 755 Annex C) — decodes
  the previously-raw 22-bit signalling payload.
- `Bbheader::issy()` — typed accessor decoding the raw `issy_in_header` bytes.
- `CarryOverExtractor::stats()` → `CarryOverStats`: diagnostic counters
  (`npd_unsupported`, `header_parse_failures`, `mode_mismatches`,
  `partial_discards`) making the extractor's resilient skips observable.
  `npd_unsupported` flags valid HEM frames dropped because NPD/DNP reinsertion is
  unsupported. The `feed_*` API stays infallible.

### Changed
- `decode_issy_short` / `decode_issy_long` return `Result` (was `Option`) — a
  form/prefix mismatch is now a diagnosable error; ISSY bit-masks are named consts.
- `up_iter()` returns a concrete iterator (was `Box<dyn Iterator>` — drops the
  heap alloc); `remaining()` is panic-free on an out-of-range cursor.

## [5.0.0] — 2026-06-11

Lockstep major across the workspace. `dvb-bbframe` adopts the shared
`Parse`/`Serialize` contract and the unified error model, and hardens the
user-packet extractor against wire-derived input.

### Added
- **`Bbheader` implements the workspace `dvb_common::Parse` / `Serialize`
  contract** (#106) — symmetric, byte-identical round-trip — alongside the
  existing inherent `parse` / `serialize` convenience methods.

### Changed
- **Unified cross-crate error model** (#112): `Error` is now structured
  `thiserror` variants consistent with the rest of the workspace. (Breaking:
  error type / variants changed.)
- **`Issy` is now `#[non_exhaustive]`** (#106) — matching it requires a
  wildcard arm. (Breaking for exhaustive matches.)

### Fixed
- `CarryOverExtractor::feed_hem_into` / `feed_nm_into` no longer panic on
  wire-derived input (a `no_payload_data` HEM packet, or a header whose mode
  doesn't match the called path); they emit no packets for that frame instead
  of asserting.
- Magic `0x47` sync-byte literals replaced with the named `TS_SYNC_BYTE`.

## [4.3.0] — 2026-06-10

### Added
- `CarryOverExtractor::feed_hem_into` / `feed_nm_into` — buffer-reusing variants
  that append completed TS packets into a caller-provided `Vec` (cleared each
  call), avoiding the per-frame allocation of `feed_hem`/`feed_nm` (now thin
  wrappers). Behaviour unchanged (#45).

## [4.2.0] — 2026-06-09

Version-lockstep release with the workspace (dvb-si DSM-CC `ModuleReassembler`
hardening, #42 / #43); no changes to this crate.

## [4.1.0] — 2026-06-09

Version-lockstep release with the workspace (`dvb-si` decoded accessors); no
changes to this crate.

## [4.0.0] — 2026-06-08

Version-lockstep release with the workspace (the `dvb-si` 4.0 section/table
split — `*Section` parsers and the new `collect` module); no changes to this
crate.

## [3.1.2] — 2026-06-07

Version-lockstep release with the workspace (dvb-si spanning-into-PUSI section fix); no changes to this crate.

## [3.1.1] — 2026-06-07

Version-lockstep release with the workspace (dvb-si `SectionReassembler`
concatenated-section fix); no changes to this crate.

All notable changes to the `dvb_bbframe` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.1.0] — 2026-06-05

Lockstep release with the `dvb-si` 3.x line.

### Breaking
- **serde is Serialize-only.** Every `Deserialize` derive is removed from
  `Bbheader`, `Issy`, and the matype/mode types. JSON is a display/export
  format; re-`parse` from wire bytes to reconstruct. `Serialize` output is
  unchanged.

### Added
- **`yoke` feature (off by default, additive).** `yoke::Yokeable` is derived on
  the public borrowing user-packet iterator view types `NmTsIter<'a>` and
  `HemTsIter<'a>` (both also gain `Clone`/`Copy`), so a parsed view can be
  retained past the input buffer's borrow without a re-parse. (`Bbheader`, the
  crate's actual parse output, is a fully owned `Copy` struct with no lifetime
  and needs no yoking.) Optional; adds no dependencies to default builds.

## [2.1.0] — 2026-06-05

Version-lockstep release with the workspace (Any* `name()` additions in
dvb-si / dvb-t2mi); no functional changes.

## [2.0.0] — 2026-06-05

Version-lockstep release with the workspace (dvb-si 2.0 typed client API);
no functional changes.

## [1.1.0] — 2026-06-04

Version-lockstep release with the workspace (dvb-si: complete EN 300 468
Table 12 descriptor coverage); doc-comment formatting/link cleanup only,
no functional changes.

## [1.0.1] — 2026-06-04

Version-lockstep release with the workspace (dvb-si README overhaul); no code changes.

## [1.0.0] — 2026-06-04

### Fixed

- Removed the fictitious "HEM CRC-8 init = 0xB5" model (`CRC8_INIT_DVB_T2`,
  `crc8_with_init`). EN 302 755 §5.1.7 defines the wire byte as
  `crc8(header) XOR MODE`; 0xB5 was init-0 propagated through exactly nine
  zero bytes and only coincided for 9-byte inputs. Parse now uses the spec
  formula directly; the `Crc8Mismatch` error variant (unreachable by
  construction — the XOR already constrains the byte) was removed. The only
  integrity signal the spec's scheme supports is `InvalidMode`.

## [0.1.0] — internal

Initial release. DVB-S2 / S2X / T2 Base-Band Frame (BBFRAME) header parser and
builder, Normal Mode (NM) and High Efficiency Mode (HEM).

### Added

- **`Bbheader`** — the 10-byte BBHEADER (MATYPE-1/2, UPL, DFL, SYNC, SYNCD,
  CRC-8) with `Parse` + `Serialize` round-trip (CRC-8 recomputed on serialize).
- **`Mode`** (NM/HEM) recovered from `computed_crc8 ^ stored_byte`
  (EN 302 307-1 §5.1.4 / EN 302 755 Annex F).
- **`Matype`** / **`TsGs`** decoding — SIS/MIS, CCM/ACM, ISSYI, NPD, ext, ISI.
- **`packet::up_iter`** — user-packet extraction from the data field honouring
  SYNCD and the UPL stride.
- **`issy`** — ISSY (Input Stream Synchronizer) field parser, EN 302 755 Annex C.
- **`serde` feature** — optional `Serialize`/`Deserialize` derives on the public
  data types (off by default).
- **Error handling** — `Error` enum with `thiserror`. The `DflOutOfRange`
  ceiling (`DFL_MAX_BITS` = 64800) is the DVB-S2 normal-FECFRAME bound
  (EN 302 307-1 §5.1.4); DVB-T2 is tighter (EN 302 755 Table 2).

### Known Limitations

- **Physical layer out of scope** — LDPC/BCH coding and modulation are not
  implemented; this crate handles the BBHEADER and data-field framing only.
- **No `no_std` support** — `thiserror` requires `std`.
