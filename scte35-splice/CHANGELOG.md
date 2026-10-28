# Changelog

## [Unreleased]
### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

## [4.0.0] - 2026-10-17
### Documentation
- README and crate-root docs restructured: what it does / does not, standards table (SCTE 35 2023r1, ETSI TS 103 752-1 V1.2.1), command/descriptor coverage matrix, feature table.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Fixed (adversarial review of signalling/captions)
- **BREAKING (behavioural):** `EquivalentSegmentationType::bits()` / `TimelineType::bits()` return a `Reserved(v)` verbatim instead of masking it to 4 bits, and the compact `splice_insert` DAS block, the stand-alone `DVB_DAS_descriptor()` and `DSM-CC_stream_event_payload_binary()` serializers reject a value above `0xF` with `FieldOverflow` (it used to be written as a different valid-looking value, e.g. `Reserved(0x12)` as `ProviderPlacementOpportunity` / `Temi`).
- **BREAKING (behavioural):** `StreamEventPayload::serialize_into` requires the `temi_*` bytes exactly when the wire `timeline_type` is `0x2` (`{Temi, temi: None}` and `{VideoPts, temi: Some}` are rejected instead of producing output the parser rejects or silently loses).
- **BREAKING (additive field on a literal-constructible struct):** `StreamEventPayload` gains `reserved_dvb_data: &[u8]`, the `reserved_zero_future_use` run inside `DVB_data_length` that parse used to skip and serialize used to drop; it is now preserved so such a payload re-serializes byte-exactly (`&[]` for hand-built payloads).
- **BREAKING (behavioural):** `CompactTimeSignal` / `CompactSpliceInsert` serialize rejects `encrypted_packet: true` with `e_crc_32: None` (it wrote `E_CRC_32 = 0`, which re-parsed as `Some(0)`) and `e_crc_32: Some` when not encrypted (silently dropped).

### Fixed
- Serialize: the DVB-TA DAS `reserved` nibble in the compact encoder was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

### Added (item 3 - DVB DAS nibble-order ambiguity, ETSI TS 103 752-1)
- `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8, `TypeLow` = Table 1) plus `DvbDasDescriptor::{parse,serialize_into}_with_order` and `CompactSpliceInsert::{parse,serialize_into}_with_order`: Table 1 (stand-alone `DVB_DAS_descriptor()`, type in the low nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree about the `equivalent_segmentation_type`/`reserved` nibble order of the same body. After searching for a resolution (no newer edition or corrigendum, no byte examples in TS 103 752-1/TR 103 752-2/A178-1r1, no independent implementation, no real capture) the question stays open: default wire behaviour is unchanged (descriptor: Table 1, compact: Table 8), both readings are selectable and pinned by `tests/das_nibble_order.rs` (revert-verified), and the evidence is recorded in `docs/dvb_ta/compact-scte35.md` and the `DasNibbleOrder` docs.

### Fixed (#1204)
- A clear `splice_info_section` whose `splice_command_length` is the deprecated `0xFFF` "backwards compatibility" value is no longer rejected. SCTE 35 §9.6.1 says that value "shall be ignored by downstream equipment", so `parse` now derives the command's extent from its own structure (the smallest length that both parses as the command and closes the descriptor loop exactly at the CRC; an underivable layout, e.g. with alignment stuffing, is still an error, never a guess). Found with a real capture (TSDuck `test-091.ts`, `tests/real_capture.rs`). **BREAKING (additive field on a literal-constructible struct):** `ClearPayload` gains `legacy_command_length_sentinel: bool`; `serialize_into` writes `0xFFF` back when set so such a section round-trips byte-for-byte (`false` and the real length for conformant / hand-built sections).

### Added (#1204)
- `tests/real_capture.rs`: real `splice_info_section`s from the committed `fixtures/ts/scte35-*.ts` / SSAI fixture (always) and the local-only TSDuck corpus (archive-gated, skip loudly), checked against an independent Python decode of the spec syntax and re-serialized byte-exactly.
- `tests/dvb_ta_compact_vectors.rs` + `tests/fixtures/dvb_ta/gen_compact_vectors.py`: independent-oracle byte vectors for the DVB TA compact `time_signal` / `splice_insert` (TS 103 752-1 §8.3.3); documents the open DAS-nibble-order question (`docs/dvb_ta/compact-scte35.md`).

### Changed (BREAKING, #1202)
- **BREAKING (behavioural):** `SpliceInfoSection::serialize_into` rejects `sap_type` > 3 and `encryption_algorithm` > 63, and `AudioDescriptor::serialize_into` rejects `bit_stream_mode` > 7 / `num_channels` > 15, with `Error::FieldOverflow` instead of masking them into a different valid-looking value (the #1129 class). Guarded by `tests/serialize_mask_guard.rs` and `tests/serialize_field_width.rs`; `tests/hostility.rs` sweeps CRC-repaired mutations of real sections for panics (none found).

### Changed (BREAKING)
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

### Fixed
- `time::ticks_to_duration` saturates instead of wrapping for an out-of-field
  tick count (#1102 O1).

## [3.0.1] - 2026-10-13
### Changed

- `PTS_MODULUS`/`pts_add_wrapping` and the DVB-TA 33-bit mask now delegate to `broadcast_common::clock33` (no behaviour change for valid input); added `tests/clock33_guard.rs`.

## [3.0.0] - 2026-10-05
### Changed (breaking)
- Serializers now return an error, instead of silently truncating, when a
  length, count, or out-of-range field value does not fit its wire field
  (#1129). Requires `broadcast-common` 9.4 (`broadcast_common::len`). A new
  `Error::FieldOverflow` variant is added (the enum is `#[non_exhaustive]`,
  but this is still a behavior change worth calling out for a crate at
  2.x): `splice_descriptor::header::write_header` now returns `Result<()>`
  instead of `()`.
- `SpliceInfoSection` gains `encrypted_splice_command_length: Option<u16>`
  (#1102 W1) and `ClearPayload` gains `alignment_stuffing: &'a [u8]`
  (#1102 W2), both required to construct these types directly (parse fills
  them in automatically).

### Fixed
- `SpliceInfoSection::serialize_into` wrote the deprecated `0xFFF`
  "ignore" sentinel for an encrypted section's `splice_command_length`
  instead of the real value — readable without decrypting anything, since
  it sits before `splice_command_type` — even though `parse` already
  captured it. This broke the module's own documented byte-for-byte
  round-trip claim, and this crate's own clear-path parser rejects the
  sentinel, so decrypting-then-reparsing a section this crate re-serialized
  failed (#1102 W1).
- `alignment_stuffing()` (padding bytes between the descriptor loop and
  `CRC_32`, present on every section, not only encrypted ones) was silently
  dropped on parse and never re-emitted on serialize, despite a comment
  claiming it was "re-derived on serialize" — breaking byte-identical
  round-trip for any section carrying it (#1102 W2).
- The DVB-TA compact-encoding module's (`dvb_ta::compact`) reconstructed
  bit widths were already disclosed as "likely" in the module doc, but not
  as prominently as the underlying risk warrants — no real DVB TA
  compact-watermark capture or authoritative erratum has been obtained to
  confirm them, so a wrong width would silently misdecode rather than
  error. Strengthened the module doc with an explicit, unmissable warning
  (#1102 W5); functionally unchanged pending a real fixture or erratum.
- `SpliceInfoSection::serialize_into` narrowed `section_length` with `as u16`
  before the `> 4093` range check, and narrowed `splice_command_length`/
  `descriptor_loop_length` with unchecked `as u16` casts, so a command body or
  descriptor loop of 64 KiB or more silently wrapped into a short, misframed
  section instead of erroring (#1129).
- `SpliceSchedule`/`SpliceInsert` component and event counts, and the
  `AnySpliceDescriptor::Unknown` and generic `splice_descriptor()` header
  `descriptor_length`, were written with unchecked `as u8` casts, wrapping to
  0 for 256+ items while all items were still serialized (#1129).
- `SpliceTime`/`BreakDuration`/`SegmentationDescriptor` component
  `pts_offset`/`segmentation_duration`, and the DVB-TA compact `pts_time`/
  `duration` fields, silently masked an out-of-range 33-bit/40-bit value on
  serialize instead of erroring, unlike `SpliceInfoSection`'s own
  `pts_adjustment` check (#1129).

### Changed
- `dvb_ta::base64_encode` delegates to the `base64` crate (`no_std` + `alloc`); output and signature unchanged. `base64` is now a normal dependency (was a dev-dependency).

## [2.1.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
## [2.0.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

  This is a **major** bump (1.x → 2.0.0) for that reason alone: anything pinned
  to `scte35-splice` 1.x with `broadcast-common` 8 keeps working and is not
  forced to move.

## [1.1.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [1.0.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [1.0.0] — 2026-06-27

- Renamed from `dvb-scte35` (was 7.9.0); code unchanged. Left the DVB lockstep — now versioned independently. Old crate `dvb-scte35` continues as a deprecated re-export shim at 7.9.1.

## [7.9.0] — 2026-06-22

### Changed

- Lockstep release; no functional changes to this crate.

## [7.8.0] — 2026-06-21

### Fixed
- **`DvbDasDescriptor::serialize_into` — `descriptor_length` overflow guard
  (dvb_ta audit P1):** a `upid` of ≥ 252 bytes would make `4 + body_len ≥ 256`,
  silently wrapping the 8-bit `descriptor_length` field and producing corrupt
  wire bytes with no error. `serialize_into` now returns
  `Err(Error::InvalidValue { field: "DVB_DAS_descriptor.descriptor_length", … })`
  when `FIXED_BODY_LEN + upid.len() > 251` (the largest value where
  `descriptor_length = 4 + body_len` still fits in a `u8`). Mirrors the guard
  already present in `CompactTimeSignal::serialize_into` and
  `CompactSpliceInsert::serialize_into`.

### Added
- **TOML spec-mirror + drift tests for `EquivalentSegmentationType` (Table 2) and
  `TimelineType` (Table 4)** (dvb_ta audit C): `docs/enums/dvb_ta/
  equivalent_segmentation_type.toml` and `docs/enums/dvb_ta/timeline_type.toml`
  record every named variant/value/spec-label from ETSI TS 103 752-1 Tables 2 and
  4; `tests/dvb_ta_spec_drift.rs` byte-sweeps each enum and cross-checks the TOML,
  mirroring the existing SCTE-35 enum drift tests.
- **`CompactTimeSignal` upid-overflow regression test** (dvb_ta audit 2A): a new
  in-module test `time_signal_upid_overflow_guard_fires` confirms the existing
  guard at `compact.rs` fires when `segmentation_upid.len() > 255`.

### Added
- **DVB Targeted Advertising — binary SCTE 35 profile (ETSI TS 103 752-1 V1.2.1)**
  in a new `dvb_ta` module (DVB-TA folded into this crate, not a new crate, since
  it is a DVB profile over ANSI/SCTE 35):
  - `dvb_ta::das_descriptor::DvbDasDescriptor` — the `DVB_DAS_descriptor()`
    (§5.3.5.16, tag `0xF0`, identifier `"DVB_"`): a DVB-private splice descriptor
    carried in the base `splice_info_section()` descriptor loop, with
    `EquivalentSegmentationType` (Table 2). Integrated into
    `AnySpliceDescriptor` (dispatched on tag `0xF0`).
  - `dvb_ta::compact` — the Compact SCTE 35 Encoding Format (§8.3.3):
    `CompactScte35` / `CompactTimeSignal` / `CompactSpliceInsert` (+ inline
    `CompactDas`) for low-capacity watermark carriage. Implements the
    reconstructed ("likely") bit widths the transcription cross-checks against
    §5.3.5.11 and Table 1 (the PDF render of Tables 6/8 is mis-registered).
  - `dvb_ta::stream_event::StreamEventPayload` — the
    `DSM-CC_stream_event_payload_binary()` (§6.3.1) wrapping an inline
    `SpliceInfoSection` (`event_type 0`) or a carousel-object DVB URI reference
    (`event_type 1`), with `TimelineType` (Table 4) and a TEMI option; plus a
    `no_std` RFC 4648 `base64_encode` helper for the carriage recipe.
  - `dvb_ta::profiling` — typed helpers over the existing
    `SegmentationDescriptor` for the PPO/DPO profiling constraints (§5.3.4–5.3.5,
    no new wire syntax): `PlacementOpportunity` (classifies the 0x34–0x37 PO
    type-IDs) and `validate_po_segmentation` → `ProfileViolation`.
  - The base SCTE 35 structures the profile references
    (`splice_info_section()`, `splice_insert()`, `time_signal()`,
    `segmentation_descriptor()`, the CRC-32 and the `segmentation_type_id` /
    `segmentation_upid_type` tables) are reused, not reimplemented.

## [7.7.1] — 2026-06-21

### Changed
- Value-enum accessors (`to_u8`/`to_u16`/`to_u32`) are now `const fn`, so they can be used in `const` initializers downstream. Non-breaking.

## [7.7.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.6.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.5.0] — 2026-06-19

### Added
- `examples/`: `parse_splice_insert` (parse a real `splice_insert` section) and
  `round_trip_and_descriptors` (walk the descriptor loop + byte-exact round-trip).

## [7.4.0] — 2026-06-18

Lockstep release; no functional changes.

## [7.3.0] — 2026-06-17

### Changed
- Lockstep release; no functional changes to this crate.

## [7.2.0] — 2026-06-16

### Fixed
- `SpliceInfoSection::parse` no longer underflows (`total - CRC_LEN`) / panics on
  a section with `section_length < 1`; guarded before the subtraction (#216).

## [7.1.0] — 2026-06-15

### Changed
- Internal: byte-readers hardened with bounds-checked `split_first_chunk` /
  `split_last_chunk` / `first_chunk`, folding manual length checks into the read.
  No API or behaviour change (#207).

## [7.0.0] — 2026-06-14

**BREAKING (MSRV 1.75 → 1.81).**

### Added
- **no_std + alloc support** (#63).

### Changed (breaking)
- Every public enum is `#[non_exhaustive]` (#208) — incl. `DeviceRestrictions`,
  `SegmentationUpidType`, `SegmentationTypeId`.
- MSRV **1.81**.

## [6.7.0] — 2026-06-14

### Added
- `name()` + `Display` on `DeviceRestrictions`, `SegmentationUpidType`
  (Table 22 UPID names) and `SegmentationTypeId` (Table 23 names) via
  `impl_spec_display!`; `label_coverage` drift-guard test (#204).

## [6.6.0] — 2026-06-14

Version-lockstep release with the workspace (dvb-t2mi L1-pre/L1-post signalling parser #54; dvb-si BIOP object-carousel layer #64; criterion benchmark suites #62). No changes to this crate.

## [6.5.0] — 2026-06-13

Version-lockstep release with the workspace (#47 T2 emission-time accessors; #50 SSU GroupInfoIndication + data_broadcast_id 0x000A selector; #53 S2Xv2 0x24 extension descriptor). No changes to this crate.

## [6.4.0] — 2026-06-13

### Added
- Spec-table data mirror `dvb-scte35/spec_tables/segmentation_type_id.toml`
  (reviewable, spec-cited `segmentation_type_id` value→name table) plus
  `tests/spec_drift.rs`, a drift test that byte-sweeps `SegmentationTypeId`
  and fails CI if the enum and its TOML diverge (#158). Test/data only.
- Extended `tests/spec_drift.rs` with drift-guard coverage for three further
  code-backing enums/dispatch tables (#158): `DeviceRestrictions` (Table 21,
  4 variants), `SegmentationUpidType` (Table 22, 18 named variants) and
  `splice_command_type` via `AnyCommand::DISPATCHED_TYPES` (Table 7, 6
  implemented types). Adds `spec_tables/device_restrictions.toml`,
  `spec_tables/segmentation_upid_type.toml`, and
  `spec_tables/splice_command_type.toml`. Test/data only.

## [6.3.0] — 2026-06-13

_Initial release._

### Added
- **Typed MPU/MID UPID sub-structures** (§10.3.3.3-4, Tables 24-25): `Mpu<'a>`
  (format_identifier + private_data) and `MidUpid<'a>` (per-entry type + raw upid
  bytes) decoded on demand via `SegmentationDescriptor::mpu()` /
  `SegmentationDescriptor::mid()`. Raw `segmentation_upid: &[u8]` is unchanged so
  round-trip serialization is unaffected.

### Fixed
- **Serde test vector**: replaced the self-assembled base64 in
  `tests/serde_round_trip.rs` with the real ANSI/SCTE 35 2023r1 §14.1 vector
  (`/DA0AAAAAAAA///wBQb+…`); assertions updated to match the spec-decoded fields.

- **New crate `dvb-scte35`** (#58) — ANSI/SCTE 35 2023r1 splice information
  (Digital Program Insertion cueing) parser **and** builder, with the
  workspace's symmetric `Parse`/`Serialize` round-trip discipline.
  - `SpliceInfoSection` (table_id `0xFC`): the full §9.6 header
    (protocol_version, sap_type, the encryption flags, 33-bit `pts_adjustment`,
    `cw_index`, 12-bit `tier`, `splice_command_length`/type, descriptor loop,
    CRC_32 via `dvb_common::crc32_mpeg2`). Encrypted sections are kept raw and
    round-trip losslessly; clear sections expose typed commands and descriptors.
  - Commands: `splice_null`, `splice_schedule`, `splice_insert`, `time_signal`,
    `bandwidth_reservation`, `private_command`, plus `splice_time()` /
    `break_duration()` — unified by `AnyCommand` with a raw fall-through for
    reserved command types.
  - Splice descriptors: `avail`, `DTMF`, `segmentation` (with the
    `SegmentationTypeId` / `SegmentationUpidType` / `DeviceRestrictions`
    assignment-table enums), `time`, `audio` — unified by `AnySpliceDescriptor`
    with a raw fall-through for unknown tags.
  - Decoded accessors (the 4.1.0 pattern): 90 kHz fields (`pts_time`,
    `break_duration`, `pts_adjustment`) ⇄ `core::time::Duration`, and the 33-bit
    `pts_adjustment` carry-ignored wrap (`pts_add_wrapping`).
  - Optional `serde` feature (default on), Serialize-only — mirrors the
    workspace posture (no Deserialize).
