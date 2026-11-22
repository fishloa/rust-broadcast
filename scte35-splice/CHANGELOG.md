# Changelog

## [Unreleased]
### Tests
- Tests decode base64 through `broadcast_common::base64`; the `base64` dev-dependency is removed (#3957).
### Changed
- Test-only: a segmentation descriptor carrying all-32-bit `segmentation_event_id`, a 40-bit `segmentation_duration`, a two-entry MID() UPID, two distinct components and the sub-segment appendix is asserted byte-for-byte against hand-assembled Table 20 bytes, each optional section's truncation is a typed error, and the MID() walker's truncated-header and over-long-entry cases are pinned (audit 21 finding 18, #2913).
- Test-only: `splice_info_section()` packs a nonzero 12-bit `tier` beside a 12-bit `splice_command_length` above 255 and a `descriptor_loop_length` above 255, asserted on bytes 10-12, with `tier_is_ignored()` checked for both values, the encrypted-section packing exercised, and every truncation boundary refused (audit 21 finding 19, #2914).
- Test-only: a `splice_schedule` with three events (program, cancelled, component mode) and a `splice_insert` in scheduled program mode are asserted byte-for-byte against hand-assembled Table 9/Table 10 bytes, including the per-event `event_id_compliance_flag` and every cursor offset, plus the two empty commands' parse behaviour (audit 21 finding 20, #2915).
- Test-only: `SpliceTracker` closing one open event keeps the others in the eviction order (the open set still caps at `MAX_OPEN_EVENTS` after a close), a different section for the same event is a duplicate open while identical bytes are a retransmission, and the `NoCue`/`SpliceSanity` labels are pinned (audit 21 finding 22, #2917).
- Test-only: the DVB-TA stream-event payload's optional sections are round-tripped and truncated at every byte, its declared lengths are checked at their bounds, the compact time-signal and splice-insert bit widths and `bit_err` mapping are pinned at their maxima and maxima plus one, and the DAS `descriptor_length` gate and nibble order are asserted (audit 21 finding 23, #2918).
- Test-only: the avail/DTMF/time/audio descriptors are serialized at their maximum sizes (7 DTMF chars, 15 audio components, a 48-bit TAI second) and their one-past-the-limit rejection, the shared descriptor header gate is checked for each truncation shape, and the descriptor loop's raw iterator, `Deref`, `From` and serde paths are pinned (audit 21 finding 21, #2916).
- Test-only: every SCTE 35 and DVB-TA spec/field enum's `name()`/`Display` labels are asserted literally (including five Table 23 segmentation type labels and all Table 21/22 variants), so a label can no longer render empty or as a placeholder (audit 21 finding 24, #2919).

## [7.0.0] - 2026-11-15
### Changed
- BREAKING: `ClearPayload::descriptor_loop` is `SpliceDescriptorLoop` (new module `scte35_splice::descriptor_loop`) instead of `&[u8]`, and `SpliceInfoSection::new_clear` takes one. The loop wraps the wire bytes verbatim (`raw()`), walks the TLV structure with `entries()` (a truncated entry is one `Err` with the loop offset, then the walk fuses, instead of the bytes being hand-parsed), and `typed()` is the dispatch `SpliceInfoSection::descriptors` already performed. Wire bytes are unchanged. Migration: `ClearPayload { descriptor_loop: &bytes, .. }` -> `ClearPayload { descriptor_loop: SpliceDescriptorLoop::new(&bytes), .. }`, `new_clear(cmd, &[])` -> `new_clear(cmd, SpliceDescriptorLoop::new(&[]))`, `section.clear.unwrap().descriptor_loop` -> `...descriptor_loop.raw()` where the bytes are wanted (#2770).
- BREAKING: `SpliceObservation` (returned by `SpliceTracker::feed`) is `#[non_exhaustive]`, so a field added later is not a breaking change (#2771).
- BREAKING: `SpliceTracker::feed` takes the packet's `ContinuityCounter` and adaptation-field `discontinuity_indicator` and returns `FeedOutcome` (`Result<(), NoCue>`) instead of `()` (audit 19b, finding 16). It fed the reassembler without continuity handling, so a legally repeated TS packet (ISO/IEC 13818-1 §2.4.3.3) doubled the section bytes into a corrupt one, and a section that failed CRC or parse vanished in a `continue` with no count or event. A non-cue section now comes back as `NoCue::Malformed { raw, error }` or `NoCue::NotACue`. Migration: `feed(payload, pusi, on_event)` becomes `feed(payload, pusi, continuity_counter, discontinuity, on_event)`; a caller that ignores the outcome writes `let _ = tracker.feed(..)`.
- BREAKING: the `MAX_OPEN_EVENTS` cap evicts the oldest open event instead of the numerically smallest `splice_event_id` (the `BTreeSet::pop_first` it used), which deleted a recent, low id whenever ids wrapped or a producer's numbering started low.
- `NoCue`, `FeedOutcome`, `NoCue::Malformed`'s `raw`/`error` and the packet-level duplicate rule: `media-doctor check` reports a corrupt cue as a `scte35-malformed` Error finding instead of dropping it.
- BREAKING: `SegmentationDescriptor::sub_segments` is `Option<SubSegments>` and `SegmentationDescriptor` gains `trailing: &[u8]` (audit 19b, finding 15). Parse used to take any two bytes left over inside `descriptor_length` as `(sub_segment_num, sub_segments_expected)` and skip a third (`let _ = pos`), so the pair was guessed for a `segmentation_type_id` that does not carry it and a longer tail lost bytes on re-serialize. The appendix is now read only for the eight Table 23 `Start` types (`SegmentationTypeId::has_sub_segments`), every byte it does not claim is carried verbatim in `trailing`, and an over-long descriptor is byte-exact through parse → serialize. Building `sub_segments: Some(..)` on a type that does not carry the pair is rejected with `Error::InvalidValue { field: "segmentation_descriptor.sub_segments" }` instead of writing bytes a receiver would read as a different field.
- BREAKING: `SpliceInsert`'s parallel flags and options are one `mode: SpliceMode`: `Program(ProgramSplice::Immediate | ProgramSplice::At(SpliceTime))` or `Components { immediate, components }` (audit 19b, finding 14). `program_splice_flag`, `splice_immediate_flag`, `splice_time` and `components` are gone from the struct; the wire flags are derived from the mode, so a populated field can no longer contradict a flag. Before, `splice_immediate_flag = true` beside `splice_time: Some(t)` serialized as an immediate cue with `t` silently dropped (35-byte scheduled command became a 20-byte immediate one), and `splice_immediate_flag = false` beside `splice_time: None` serialized a time-less `splice_time()`. Migration: `program_splice_flag: true, splice_immediate_flag: true` becomes `mode: SpliceMode::Program(ProgramSplice::Immediate)`; a scheduled program cue becomes `mode: SpliceMode::Program(ProgramSplice::At(splice_time))`; component mode becomes `mode: SpliceMode::Components { immediate, components }`.
- BREAKING: `splice_event_cancel_indicator: bool` is `cancel: Option<u32>`, the id the cancel names. Parsing fills it with `splice_event_id` (the wire has one event id); build it with `SpliceInsert::cancel(id)`, and `serialize_into` rejects a `cancel` that disagrees with `splice_event_id` with `Error::InvalidField` instead of writing a cancel of the other event.
- `SpliceInsert` and `SpliceInsertComponent` keep struct-literal construction; `SpliceInsertComponent::new(tag, splice_time)` builds an entry.
- BREAKING: `SpliceTime::pts_time` is `Option<broadcast_common::clock33::Clock33>` instead of `Option<u64>`, and `SpliceTime::with_pts(u64)` is replaced by `SpliceTime::at(Clock33)` with a matching `SpliceTime::immediate()` for the `time_specified_flag == 0` form (audit 19b, finding 17). `with_pts` used to mask its argument with `& PTS_MAX` while `serialize_into` rejected anything above `PTS_MAX`: one field with two policies, so a value the constructor accepted could not be written. The field now cannot hold an off-clock value at all. `SpliceTime::parse` reports a `pts_time` wider than 33 bits, which used to be masked back onto the clock, as `Error::InvalidValue { field: "splice_time.pts_time" }`. `SpliceTime` is `#[non_exhaustive]`: build it with `at`, `immediate` or `Default`, and recover the raw tick count with `pts_time.map(|c| c.ticks())`.


## [6.0.0] - 2026-11-14
### Added
- `SpliceEvent`, `splice_event`, `judge_splice_time`, `kind_label` and `SpliceSanity`: the cue-sanity engine (cue direction, effective `pts_time + pts_adjustment`, wrap-safe future-vs-past judgement) both live TS monitors now share.
- `SpliceTracker::observe_section` and `SpliceObservation::event`, so a caller that reassembles its own sections gets the same open/closed tracking and the same decoded cue as the payload-level `feed`.
- Shared `SpliceTracker` (moved from media-doctor) for SCTE-35 splice_insert open-event tracking.

### Fixed
- A typed splice command or descriptor whose declared length exceeds its structure (excess bytes in `splice_null`, `time_signal`, `avail_descriptor`, ...) is now rejected with `Error::InvalidField` instead of being accepted and silently shortened on serialize, which broke the parse-serialize byte round trip.
- `SpliceInfoSection::parse` rejects a set `section_syntax_indicator` or `private_indicator` (fixed at 0 by §9.6.1, Table 5) instead of re-serializing the section with those bits cleared.

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.
- New sibling dependencies: `mpeg-ts` 0.8.

## [5.0.0] - 2026-11-07
### Removed
- BREAKING: Unused public items, with no use in the workspace beyond their own unit tests: `duration_value`, `pts_adjustment_duration`, `pts_time_duration`, `set_duration_value`, `set_pts_adjustment_duration`, `set_pts_time_duration`.
### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).
### Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- BREAKING: `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- Length pre-checks go through `broadcast_common::len::{need_in, need_out}`; the error values are unchanged (c3 S1).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
- BREAKING: `Error::InvalidValue { field, reason }` (no value) is replaced. `Error::InvalidValue` now wraps `broadcast_common::error::InvalidValue { field, value, reason }` and is used for unknown/forbidden parsed values (`compact_SCTE_35.message_type`, `compact_splice_insert.descriptor_length`, `private_data_length`); a serializer value above its field's range returns `Error::FieldOverflow { field, value, max }` (33-bit PTS/duration fields, 40-bit duration, 48-bit `tai_seconds`, `tier`, `section_length`, counts and length bytes); structural problems with no single number return `Error::InvalidField { field, reason }` (`splice_command_length` 0xFFF sentinel, empty section body, `e_crc_32` presence, TEMI presence). New variant: `InvalidField`.
- BREAKING: error variants `BufferTooShort`, `OutputBufferTooSmall`, `CrcMismatch` now wrap `broadcast_common::error` types (tuple variants; `CrcMismatch` gains `what` = "splice_info_section", message "<what> CRC-32 mismatch: ..."). Hand-written `From` impls removed. `InvalidValue` stays crate-specific (carries no offending value).
### Added
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type, producing the identical existing variants (c3 S1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11. The `Cargo.toml` requirements name the new versions; no other manifest change.

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
- `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8, `TypeLow` = Table 1) plus `DvbDasDescriptor::{parse,serialize_into}_with_order` and `CompactSpliceInsert::{parse,serialize_into}_with_order`: Table 1 (stand-alone `DVB_DAS_descriptor()`, type in the low nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree about the `equivalent_segmentation_type`/`reserved` nibble order of the same body. After searching for a resolution (no newer edition or corrigendum, no byte examples in TS 103 752-1/TR 103 752-2/A178-1r1, no independent implementation, no real capture) the question stays open: default wire behaviour is unchanged (descriptor: Table 1, compact: Table 8), both readings are selectable and pinned by `tests/das_nibble_order.rs` (revert-verified), and the evidence is recorded in `private/specs-verified/scte/scte35-splice/dvb_ta/compact-scte35.md` and the `DasNibbleOrder` docs.

### Fixed (#1204)
- A clear `splice_info_section` whose `splice_command_length` is the deprecated `0xFFF` "backwards compatibility" value is no longer rejected. SCTE 35 §9.6.1 says that value "shall be ignored by downstream equipment", so `parse` now derives the command's extent from its own structure (the smallest length that both parses as the command and closes the descriptor loop exactly at the CRC; an underivable layout, e.g. with alignment stuffing, is still an error, never a guess). Found with a real capture (TSDuck `test-091.ts`, `tests/real_capture.rs`). **BREAKING (additive field on a literal-constructible struct):** `ClearPayload` gains `legacy_command_length_sentinel: bool`; `serialize_into` writes `0xFFF` back when set so such a section round-trips byte-for-byte (`false` and the real length for conformant / hand-built sections).

### Added (#1204)
- `tests/real_capture.rs`: real `splice_info_section`s from the committed `fixtures/ts/scte35-*.ts` / SSAI fixture (always) and the local-only TSDuck corpus (archive-gated, skip loudly), checked against an independent Python decode of the spec syntax and re-serialized byte-exactly.
- `tests/dvb_ta_compact_vectors.rs` + `../fixtures/scte35/dvb_ta/gen_compact_vectors.py`: independent-oracle byte vectors for the DVB TA compact `time_signal` / `splice_insert` (TS 103 752-1 §8.3.3); documents the open DAS-nibble-order question (`private/specs-verified/scte/scte35-splice/dvb_ta/compact-scte35.md`).

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
  `TimelineType` (Table 4)** (dvb_ta audit C): `private/specs-verified/scte/scte35-splice/enums/dvb_ta/
  equivalent_segmentation_type.toml` and `private/specs-verified/scte/scte35-splice/enums/dvb_ta/timeline_type.toml`
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
