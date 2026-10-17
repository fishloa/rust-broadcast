# mpeg-pes 0.6.0

_Released 2026-10-17._

**Breaking release.** Everything `PesPacket::parse` used to accept but `serialize` silently rewrote is now a typed, preserved field, so parse -> serialize is byte-exact for non-conformant input (a real TSDuck stream has a `0` leading ES_rate marker bit that the writer forced to `1`): header stuffing is kept verbatim (`PesHeader::header_stuffing: &[u8]`), `PesExtension` and `PesHeader` keep every reserved and marker bit, and `additional_copy_info`, `Escr` and `TrickMode::FreezeFrame` keep theirs too. `PesPacket::parse` now rejects a non-`10` header marker (MPEG-1 PES or damage) and a `PES_header_data_length` that overruns a bounded `PES_packet_length`; over-range `P-STD_buffer_size` and `PES_extension` reserved bits are `FieldOverflow` errors instead of being masked. AVS3 stream-id extensions are added, and the crate builds against `broadcast-common` 10.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`PesHeader::header_stuffing_len` replaced by `header_stuffing`; new public fields and types on `PesHeader`, `PesExtension`, `Escr`, `TrickMode::FreezeFrame`; `additional_copy_info` retyped; new `Error::BadHeaderMarker`).
- **cargo-semver-checks vs the live release (all features):** MAJOR (2 major, 0 minor lint failures; note it misses the retyped `additional_copy_info`, the new `Escr` and `FreezeFrame` fields and the stricter parse, which the CHANGELOG lists). (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by `broadcast-common` 9 -> 10.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before
PesHeader { header_stuffing_len: 3, /* .. */ }
// after: the stuffing bytes themselves (non-0xFF stuffing now round-trips)
PesHeader { header_stuffing: &[0xFF; 3], /* .. */ }    // use &[] for none

// before
header.additional_copy_info = Some(0x0A);
// after: 7-bit info + marker bit; new() sets the spec marker 1
header.additional_copy_info = Some(AdditionalCopyInfo::new(0x0A));

// before
TrickMode::FreezeFrame { field_id }
// after: the 3 reserved bits are preserved; the spec-convention value is exported
TrickMode::FreezeFrame { field_id, reserved: FREEZE_FRAME_RESERVED_VALUE }   // 0b111
```

- `PesHeader` gains `es_rate_marker_bits` and `PesHeader::new()` / `Default`; `PesExtension` gains `reserved_bits`, is `#[non_exhaustive]` and is built with `PesExtension::new()` / `Default` (spec-convention `0b111`); `Escr` gains `reserved: u8` (spec `0b11`, set by `Escr::from_27mhz`). Prefer the constructors and `..Default::default()` over struct literals so future reserved fields do not break you again.
- Parse: `PesPacket::parse` returns the new `Error::BadHeaderMarker` when the first flag byte is not `10`, and rejects a `PES_header_data_length` that does not fit inside a bounded `PES_packet_length`.
- Serialize: `P-STD_buffer_size` over 13 bits and an over-range `PES_extension` `reserved_bits` return `Error::FieldOverflow` instead of being masked into a different value. The constructed `FreezeFrame` reserved bits are now `0b111`, where the old serializer wrote `000`.
- Added: `StreamId::EXTENDED_STREAM_ID` (`0xFD`), `STREAM_ID_EXTENSION_AVS3_VIDEO` / `STREAM_ID_EXTENSION_AVS3_AUDIO`, `PesExtensionField::stream_id_extension()`, and `PesExtensionField` is now re-exported.

Manifest: `broadcast-common` 9.4 -> 10.0. `Serialize::to_bytes` was removed from `broadcast-common` 10; use `try_to_bytes`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed
- Serialize: `PES_extension` `reserved_bits` was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

#### Added (#1224)
- `StreamId::EXTENDED_STREAM_ID` (0xFD), `STREAM_ID_EXTENSION_AVS3_VIDEO` (0x41) and `STREAM_ID_EXTENSION_AVS3_AUDIO` (0b100_1111) (ETSI TS 101 154 V2.10.1 section 4.1.8; T/AI 109.7 clause 8.1.2), `PesExtensionField::stream_id_extension()` accessor, and `PesExtensionField` is now re-exported.

#### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Fixed

- **`P-STD_buffer_size` over 13 bits is rejected (#1129)** instead of being masked with `& 0x1F`; `PesExtension::serialize_into` returns `Error::FieldOverflow`.

#### Changed
- ESCR field encoding uses named marker/mask constants (no behaviour change; audit r01-W15 residue).

#### Changed (BREAKING, review of the dvb-si tables/core batch)
- Three more header bits that parse accepted and serialize silently rewrote are now typed and preserved (parse tolerant, serialize preserves, hand-built default = spec value):
  - **BREAKING:** `PesHeader::additional_copy_info` is `Option<AdditionalCopyInfo>` (new, `#[non_exhaustive]`: `info` 7 bits + `marker` bit, `AdditionalCopyInfo::new(info)` sets the spec marker `1`) instead of `Option<u8>`; `...80 04 01 0A` used to come back as `0x8A`.
  - **BREAKING:** `Escr` gains `reserved: u8` - the leading `reserved(2)` of the ESCR field (spec `0b11`, set by `Escr::from_27mhz`; `04...` used to come back as `C4...`).
  - **BREAKING:** `TrickMode::FreezeFrame` gains `reserved: u8` - the 3 `reserved` bits after `field_id` (Table 2-24; `...52` used to come back as `0x50`). The constructed value is now the spec-convention `FREEZE_FRAME_RESERVED_VALUE` (`0b111`, newly exported), where the old serializer wrote `000`. Migration: `TrickMode::FreezeFrame { field_id, reserved: FREEZE_FRAME_RESERVED_VALUE }`.
- Tests: the "committed corpus" canaries (`no_committed_fixture_has_nonff_stuffing_or_111_reserved`, and `mpeg-ts`'s `every_committed_ts_adaptation_field_round_trips`) walked the whole workspace tree and skipped the private checkout by the directory NAME `private`, so a CI checkout at `private-fixtures/` was scanned as "committed" and tripped the canaries on real TSDuck data. They now skip it by identity (`test_fixtures::is_private_checkout`), and the real data is covered by positive pinned-count tests (`real_tsduck_test_002_pes_header_counts_are_pinned`, `real_tsduck_adaptation_field_extension_counts_are_pinned`).

#### Changed (breaking, umbrella #1139 T11 / audit r01-W2)
- `PesHeader::header_stuffing_len: usize` is replaced by
  `PesHeader::header_stuffing: &'a [u8]` (the stuffing bytes verbatim; use `&[]`
  for none), so non-`0xFF` stuffing is no longer rewritten to `0xFF`.
  Migration: `header_stuffing_len: n` -> `header_stuffing: &[0xFF; n]`.
- `PesExtension` gains `reserved_bits` (the 3 `reserved` bits of the extension
  flags byte, ISO/IEC 13818-1 Table 2-21), preserved on parse and written back
  (previously always written as `000`). It is now `#[non_exhaustive]` with
  `PesExtension::new()` / `Default` (spec-convention `0b111`).

- `PesHeader` gains `es_rate_marker_bits` (the two `ES_rate` marker bits; a real
  stream, TSDuck `test-002.ts`, has a `0` leading marker the writer used to
  force to `1`) and `PesHeader::new()` / `Default`.
- `PesPacket::parse` now rejects (new `Error::BadHeaderMarker`) a header whose
  first flag byte is not `'10'` (Table 2-21; MPEG-1 PES or damage was silently
  misparsed), and rejects a `PES_header_data_length` that does not fit inside a
  bounded `PES_packet_length` (it re-serialized beyond the declared length).

#### Added
- `tests/pes_header_roundtrip_exhaustive.rs`: every PES packet reassembled from
  every committed `.ts` fixture re-serializes byte-exactly; real evidence for
  header stuffing (`pes-pts-dts-stuffing.ts`) and for `reserved == 0b000`
  (`dirac-vc2-extended-stream-id.ts`); spec-built vectors for non-`0xFF`
  stuffing and all 8 reserved values.

## Merged changes since `mpeg-pes-v0.5.0`

- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- fix: dvb-si tables/core review defects + CI-exposed corpus canaries (supersedes #1270) (#1274)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- Fix the confirmed defects of the dvb-si descriptor review (supersedes #1268) (#1272)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- feat(dvb-si): AVS3 audio/video descriptors, AVS2/AVS3 stream types, extended PES ids (G1b) (#1226)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: pack_marked/unpack_marked for the 5-byte marker-bit timestamp; migrate mpeg-pes, mpeg-ts, st291, transmux (#1208) [stacked on #1207] (#1209)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
