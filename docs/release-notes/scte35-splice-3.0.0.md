# scte35-splice 3.0.0

_Released 2026-10-05._

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

---

Published from tag `scte35-splice-v3.0.0`.
