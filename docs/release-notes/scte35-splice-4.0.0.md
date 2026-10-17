---
status: DRAFT
---
# scte35-splice 4.0.0

_Released 2026-10-16._

**Breaking release (its own wire-layout and serializer changes, plus the `broadcast-common` 9 -> 10 and `mpeg-ts` 0.5 -> 0.6 epoch cascade).** Three groups of change. (1) Decode correctness: DVB TA compact `time_signal`/`splice_insert` were misdecoded (the PDF table's bit column is offset one row; the old layout inserted SCTE 35-style reserved bits the table does not list) and now follow ETSI TS 103 752-1 V1.2.1 Tables 6 and 8; a `splice_info_section` carrying the deprecated `0xFFF` `splice_command_length` (SCTE 35 section 9.6.1: "shall be ignored by downstream equipment") now parses, found with a real capture (TSDuck `test-091.ts`). (2) Serializers stop masking: over-range `sap_type`, `encryption_algorithm`, `bit_stream_mode`, `num_channels`, reserved nibbles, and inconsistent `encrypted_packet`/`e_crc_32` or `temi` combinations return `FieldOverflow`/an error instead of writing a different valid-looking value. (3) Round-trip fidelity: the `reserved_zero_future_use` run inside `DVB_data_length` is preserved in `StreamEventPayload::reserved_dvb_data`.

## Why this version

- **Proposed version:** 3.0.1 -> **4.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (ClearPayload.legacy_command_length_sentinel, compact layout fix, Error variant removed).
- **cargo-semver-checks vs live 3.0.1 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** in addition to its own breaks, builds against `broadcast-common` 9 -> 10 and `mpeg-ts` 0.5 -> 0.6. Siblings it depends on that move epoch in this wave: broadcast-common, mpeg-ts.
- **Caret epoch:** 3 -> 4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-ts = "0.5" # dev-dependency
+mpeg-ts = "0.6" # dev-dependency
```

## Breaking changes and migration

### Struct literals (additive fields)

```rust
// before
ClearPayload { .. }                         // splice_command etc.
StreamEventPayload { timeline_type, temi, .. }
// after
ClearPayload { legacy_command_length_sentinel: false, .. }
StreamEventPayload { reserved_dvb_data: &[], .. }
CompactSpliceInsert { reserved: SI_RESERVED_DEFAULT, .. }
CompactDas { reserved: DAS_RESERVED_DEFAULT, .. }
```

`ClearPayload::legacy_command_length_sentinel` makes `serialize_into` write `0xFFF` back, so such a section round-trips byte-for-byte; use `false` for conformant or hand-built sections.

### Compact DVB TA messages change meaning

`compact_time_signal()` / `compact_splice_insert()` now use the bit packing of TS 103 752-1 V1.2.1 Tables 6 and 8: no reserved bits between `encryption_algorithm`/`cw_index`/`pts_time`, a 6-bit `reserved` after `DAS_descriptor_flag`, and `equivalent_segmentation_type` (high nibble) before the 4-bit `reserved` in the DAS block. **Compact messages produced with the old layout will not decode to the same fields**; they were wrong on the wire. `Error::UnexpectedCommandType` (never constructed) is removed, so remove any match arm naming it.

Table 1 (stand-alone `DVB_DAS_descriptor()`, type in the low nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree about the nibble order of the same body, and no newer edition, corrigendum, byte example, independent implementation or real capture resolves it. The default wire behaviour is unchanged (descriptor: Table 1, compact: Table 8); both readings are selectable with the new `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8, `TypeLow` = Table 1) and `DvbDasDescriptor::{parse,serialize_into}_with_order` / `CompactSpliceInsert::{parse,serialize_into}_with_order`. The evidence is recorded in `docs/dvb_ta/compact-scte35.md`.

### Serializers reject instead of mask (`FieldOverflow`)

- `SpliceInfoSection::serialize_into`: `sap_type` > 3, `encryption_algorithm` > 63. `AudioDescriptor::serialize_into`: `bit_stream_mode` > 7, `num_channels` > 15.
- `EquivalentSegmentationType::bits()` / `TimelineType::bits()` return a `Reserved(v)` verbatim instead of masking it to 4 bits; the compact `splice_insert` DAS block, the stand-alone `DVB_DAS_descriptor()` and `DSM-CC_stream_event_payload_binary()` serializers reject a value above `0xF` (it used to be written as a different valid-looking value, e.g. `Reserved(0x12)` as `ProviderPlacementOpportunity` / `Temi`). The compact DAS `reserved` nibble is a checked `fit_mask`.
- `StreamEventPayload::serialize_into` requires the `temi_*` bytes exactly when the wire `timeline_type` is `0x2`: `{Temi, temi: None}` and `{VideoPts, temi: Some}` are rejected instead of producing output the parser rejects or silently loses.
- `CompactTimeSignal` / `CompactSpliceInsert` serialize rejects `encrypted_packet: true` with `e_crc_32: None` (it wrote `E_CRC_32 = 0`, which re-parsed as `Some(0)`) and `e_crc_32: Some` when not encrypted (silently dropped).

Values produced by `parse` always fit, so parse -> serialize round-trips are unaffected; only hand-built values can now fail.

### `broadcast-common` 10

`broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `scte35-splice` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).

Also fixed (not breaking): `time::ticks_to_duration` saturates instead of wrapping for an out-of-field tick count.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs restructured: what it does / does not, standards table (SCTE 35 2023r1, ETSI TS 103 752-1 V1.2.1), command/descriptor coverage matrix, feature table.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (adversarial review of signalling/captions)
- **BREAKING (behavioural):** `EquivalentSegmentationType::bits()` / `TimelineType::bits()` return a `Reserved(v)` verbatim instead of masking it to 4 bits, and the compact `splice_insert` DAS block, the stand-alone `DVB_DAS_descriptor()` and `DSM-CC_stream_event_payload_binary()` serializers reject a value above `0xF` with `FieldOverflow` (it used to be written as a different valid-looking value, e.g. `Reserved(0x12)` as `ProviderPlacementOpportunity` / `Temi`).
- **BREAKING (behavioural):** `StreamEventPayload::serialize_into` requires the `temi_*` bytes exactly when the wire `timeline_type` is `0x2` (`{Temi, temi: None}` and `{VideoPts, temi: Some}` are rejected instead of producing output the parser rejects or silently loses).
- **BREAKING (additive field on a literal-constructible struct):** `StreamEventPayload` gains `reserved_dvb_data: &[u8]`, the `reserved_zero_future_use` run inside `DVB_data_length` that parse used to skip and serialize used to drop; it is now preserved so such a payload re-serializes byte-exactly (`&[]` for hand-built payloads).
- **BREAKING (behavioural):** `CompactTimeSignal` / `CompactSpliceInsert` serialize rejects `encrypted_packet: true` with `e_crc_32: None` (it wrote `E_CRC_32 = 0`, which re-parsed as `Some(0)`) and `e_crc_32: Some` when not encrypted (silently dropped).

#### Fixed
- Serialize: the DVB-TA DAS `reserved` nibble in the compact encoder was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

#### Added (item 3 - DVB DAS nibble-order ambiguity, ETSI TS 103 752-1)
- `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8, `TypeLow` = Table 1) plus `DvbDasDescriptor::{parse,serialize_into}_with_order` and `CompactSpliceInsert::{parse,serialize_into}_with_order`: Table 1 (stand-alone `DVB_DAS_descriptor()`, type in the low nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree about the `equivalent_segmentation_type`/`reserved` nibble order of the same body. After searching for a resolution (no newer edition or corrigendum, no byte examples in TS 103 752-1/TR 103 752-2/A178-1r1, no independent implementation, no real capture) the question stays open: default wire behaviour is unchanged (descriptor: Table 1, compact: Table 8), both readings are selectable and pinned by `tests/das_nibble_order.rs` (revert-verified), and the evidence is recorded in `docs/dvb_ta/compact-scte35.md` and the `DasNibbleOrder` docs.

#### Fixed (#1204)
- A clear `splice_info_section` whose `splice_command_length` is the deprecated `0xFFF` "backwards compatibility" value is no longer rejected. SCTE 35 §9.6.1 says that value "shall be ignored by downstream equipment", so `parse` now derives the command's extent from its own structure (the smallest length that both parses as the command and closes the descriptor loop exactly at the CRC; an underivable layout, e.g. with alignment stuffing, is still an error, never a guess). Found with a real capture (TSDuck `test-091.ts`, `tests/real_capture.rs`). **BREAKING (additive field on a literal-constructible struct):** `ClearPayload` gains `legacy_command_length_sentinel: bool`; `serialize_into` writes `0xFFF` back when set so such a section round-trips byte-for-byte (`false` and the real length for conformant / hand-built sections).

#### Added (#1204)
- `tests/real_capture.rs`: real `splice_info_section`s from the committed `fixtures/ts/scte35-*.ts` / SSAI fixture (always) and the local-only TSDuck corpus (archive-gated, skip loudly), checked against an independent Python decode of the spec syntax and re-serialized byte-exactly.
- `tests/dvb_ta_compact_vectors.rs` + `tests/fixtures/dvb_ta/gen_compact_vectors.py`: independent-oracle byte vectors for the DVB TA compact `time_signal` / `splice_insert` (TS 103 752-1 §8.3.3); documents the open DAS-nibble-order question (`docs/dvb_ta/compact-scte35.md`).

#### Changed (BREAKING, #1202)
- **BREAKING (behavioural):** `SpliceInfoSection::serialize_into` rejects `sap_type` > 3 and `encryption_algorithm` > 63, and `AudioDescriptor::serialize_into` rejects `bit_stream_mode` > 7 / `num_channels` > 15, with `Error::FieldOverflow` instead of masking them into a different valid-looking value (the #1129 class). Guarded by `tests/serialize_mask_guard.rs` and `tests/serialize_field_width.rs`; `tests/hostility.rs` sweeps CRC-repaired mutations of real sections for panics (none found).

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

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- Fix the confirmed defects of the dvb-si descriptor review (supersedes #1268) (#1272)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- feat(scte35-splice): DasNibbleOrder - pin both readings of the DVB DAS type/reserved nibble order; record the evidence (item 3) (#1248)
- fix(scte35-splice): accept the deprecated 0xFFF splice_command_length (derive from structure, round-trip preserved); independent compact vectors; real-capture tests (#1204) (#1205)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
