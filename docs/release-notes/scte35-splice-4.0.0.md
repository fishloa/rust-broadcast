---
status: DRAFT
---
# scte35-splice 4.0.0

_Released 2026-10-16._

**Breaking release.** DVB TA compact `time_signal`/`splice_insert` were misdecoded (the PDF table's bit column is offset one row; the old layout inserted SCTE 35-style reserved bits the table does not list) and now follow TS 103 752-1 V1.2.1 Tables 6 and 8; a `splice_info_section` carrying the deprecated `0xFFF` `splice_command_length` (SCTE 35 section 9.6.1) now parses, found with a real capture (TSDuck `test-091.ts`); serializers reject over-range `sap_type`/`encryption_algorithm`/`bit_stream_mode`/`num_channels`.

## Why this version

- **Proposed version:** 3.0.1 -> **4.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (ClearPayload.legacy_command_length_sentinel, compact layout fix, Error variant removed).
- **cargo-semver-checks vs live 3.0.1 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, mpeg-ts.
- **Caret epoch:** 3 -> 4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-ts = "0.5" # dev-dependency
+mpeg-ts = "0.6" # dev-dependency
```

## Breaking changes and migration

- `ClearPayload` gains `legacy_command_length_sentinel: bool` (`serialize_into` writes `0xFFF` back when set, so such a section round-trips byte-for-byte); literal construction needs `legacy_command_length_sentinel: false`.
- `CompactSpliceInsert` and `CompactDas` gain `reserved: u8` (defaults `SI_RESERVED_DEFAULT` / `DAS_RESERVED_DEFAULT`). **Compact messages produced with the old layout will not decode to the same fields**; they were wrong on the wire.
- `Error::UnexpectedCommandType` (never constructed) is removed.
- `SpliceInfoSection::serialize_into` / `AudioDescriptor::serialize_into` return `Error::FieldOverflow` for `sap_type` > 3, `encryption_algorithm` > 63, `bit_stream_mode` > 7, `num_channels` > 15 (they were masked).
- New: `dvb_ta::DasNibbleOrder` + `*_with_order` parse/serialize for the open Table 1 vs Table 8 DAS nibble-order ambiguity (defaults unchanged).

## What changed (from CHANGELOG `[Unreleased]`)

#### Added (item 3 - DVB DAS nibble-order ambiguity, ETSI TS 103 752-1)
- `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8, `TypeLow` = Table 1) plus `DvbDasDescriptor::{parse,serialize_into}_with_order` and `CompactSpliceInsert::{parse,serialize_into}_with_order`: Table 1 (stand-alone `DVB_DAS_descriptor()`, type in the low nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree about the `equivalent_segmentation_type`/`reserved` nibble order of the same body. After searching for a resolution (no newer edition or corrigendum, no byte examples in TS 103 752-1/TR 103 752-2/A178-1r1, no independent implementation, no real capture) the question stays open: default wire behaviour is unchanged (descriptor: Table 1, compact: Table 8), both readings are selectable and pinned by `tests/das_nibble_order.rs` (revert-verified), and the evidence is recorded in `docs/dvb_ta/compact-scte35.md` and the `DasNibbleOrder` docs.

#### Fixed (#1204)
- A clear `splice_info_section` whose `splice_command_length` is the deprecated `0xFFF` "backwards compatibility" value is no longer rejected. SCTE 35 §9.6.1 says that value "shall be ignored by downstream equipment", so `parse` now derives the command's extent from its own structure (the smallest length that both parses as the command and closes the descriptor loop exactly at the CRC; an underivable layout, e.g. with alignment stuffing, is still an error, never a guess). Found with a real capture (TSDuck `test-091.ts`, `tests/real_capture.rs`). **BREAKING (additive field on a literal-constructible struct):** `ClearPayload` gains `legacy_command_length_sentinel: bool`; `serialize_into` writes `0xFFF` back when set so such a section round-trips byte-for-byte (`false` and the real length for conformant / hand-built sections).

#### Added (#1204)
- `tests/real_capture.rs`: real `splice_info_section`s from the committed `fixtures/ts/scte35-*.ts` / SSAI fixture (always) and the local-only TSDuck corpus (archive-gated, skip loudly), checked against an independent Python decode of the spec syntax and re-serialized byte-exactly.
- `tests/dvb_ta_compact_vectors.rs` + `tests/fixtures/dvb_ta/gen_compact_vectors.py`: independent-oracle byte vectors for the DVB TA compact `time_signal` / `splice_insert` (TS 103 752-1 §8.3.3); documents the open DAS-nibble-order question (`docs/dvb_ta/compact-scte35.md`).

#### Changed (BREAKING, #1202)
- **BREAKING (behavioural):** `SpliceInfoSection::serialize_into` rejects `sap_type` > 3 and `encryption_algorithm` > 63, and `AudioDescriptor::serialize_into` rejects `bit_stream_mode` > 7 / `num_channels` > 15, with `Error::FieldOverflow` instead of masking them into a different valid-looking value (the #1129 class). Guarded by `tests/serialize_mask_guard.rs` and `tests/serialize_field_width.rs`; `tests/hostility.rs` sweeps CRC-repaired mutations of real sections for panics (none found).

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).

#### Changed (BREAKING)
- `dvb_ta::compact`: `compact_time_signal()` / `compact_splice_insert()` now use
  the bit packing of ETSI TS 103 752-1 V1.2.1 Tables 6 and 8 as re-aligned from
  the PDF (the "No. of Bits" column is offset one row from `cw_index`): no
  reserved bits between `encryption_algorithm`/`cw_index`/`pts_time`, a 6-bit
  `reserved` after `DAS_descriptor_flag`, and `equivalent_segmentation_type`
  (high nibble) before the 4-bit `reserved` in the DAS block. The previous
  layout inserted SCTE 35-style reserved bits the table does not list, so every
  compact message was misdecoded (#1102 W5). `CompactSpliceInsert` gains
  `reserved: u8` and `CompactDas` gains `reserved: u8` (typed, preserved on
  round-trip; defaults `SI_RESERVED_DEFAULT` / `DAS_RESERVED_DEFAULT`).
- Removed the never-constructed `Error::UnexpectedCommandType` (#1102 O2).

#### Fixed
- `time::ticks_to_duration` saturates instead of wrapping for an out-of-field
  tick count (#1102 O1).

## Merged changes since `scte35-splice-v3.0.1`

- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- feat(scte35-splice): DasNibbleOrder - pin both readings of the DVB DAS type/reserved nibble order; record the evidence (item 3) (#1248)
- fix(scte35-splice): accept the deprecated 0xFFF splice_command_length (derive from structure, round-trip preserved); independent compact vectors; real-capture tests (#1204) (#1205)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
