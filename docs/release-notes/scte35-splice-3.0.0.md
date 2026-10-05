# scte35-splice 3.0.0

_Released 2026-10-05._

**Major (breaking).** Serializers now return an error instead of silently truncating or masking a length, count or out-of-range field, and two struct fields were added so that encrypted sections and sections with alignment stuffing round-trip byte-for-byte. Act if you call `splice_descriptor::header::write_header`, construct `SpliceInfoSection` or `ClearPayload` with struct literals, or match `Error` exhaustively. Parsing of valid input is unchanged. Consumers in this workspace that move with it: [timed-metadata 0.6.0](timed-metadata-0.6.0.md) and `ts-fix`/`compliance-probe` (see [ts-fix 0.6.0](ts-fix-0.6.0.md), [compliance-probe 0.2.0](compliance-probe-0.2.0.md)); it requires [broadcast-common 9.4.0](broadcast-common-9.4.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-base64 = "0.22"                       # was a dev-dependency (normal-dependency line removed)
+base64 = { version = "0.23", default-features = false, features = ["alloc"] }   # now a normal dependency, no_std + alloc
```

`dvb_ta::base64_encode` now delegates to the `base64` crate; its output and signature are unchanged.

## Breaking changes

1. New `Error::FieldOverflow` (wrapping `broadcast_common::len::FieldOverflow`). `Error` is `#[non_exhaustive]`, so a wildcard arm already covers it, but serializers that used to succeed on oversized input now return it (#1129).
2. `splice_descriptor::header::write_header` now returns `Result<()>` instead of `()`.

   ```rust
   // before
   write_header(buf, tag, identifier, body_len);
   // after
   write_header(buf, tag, identifier, body_len)?;
   ```

3. `SpliceInfoSection` gains `encrypted_splice_command_length: Option<u16>` (#1102 W1) and `ClearPayload` gains `alignment_stuffing: &'a [u8]` (#1102 W2). `parse` fills both. A hand-built value must set them: `None` / `&[]` reproduce the previous output for a clear section. For a hand-built encrypted section, `None` makes `serialize_into` fall back to the deprecated `0xFFF` sentinel, which this crate's own parser rejects, so supply the real length if a decrypting receiver needs it.

## Fixes

Round-trip defects (the documented byte-for-byte round-trip did not hold):

- `SpliceInfoSection::serialize_into` wrote the deprecated `0xFFF` "ignore" sentinel as an encrypted section's `splice_command_length` instead of the real value, which sits before `splice_command_type` and is readable without decrypting. Decrypting and re-parsing a section this crate had re-serialized failed (#1102 W1).
- `alignment_stuffing()` (padding between the descriptor loop and `CRC_32`, allowed on every section) was dropped on parse and never re-emitted, despite a comment saying it was re-derived on serialize (#1102 W2).

Silent truncation and masking, now errors (#1129):

- `SpliceInfoSection::serialize_into` narrowed `section_length` with `as u16` before the `> 4093` check, and `splice_command_length` / `descriptor_loop_length` with unchecked casts, so a command body or descriptor loop of 64 KiB or more wrapped into a short, misframed section.
- `SpliceSchedule` / `SpliceInsert` component and event counts, and the `AnySpliceDescriptor::Unknown` and generic `splice_descriptor()` header `descriptor_length`, were written with unchecked `as u8`, wrapping to 0 for 256 or more items while all items were still written.
- `SpliceTime`, `BreakDuration`, `SegmentationDescriptor` component `pts_offset` / `segmentation_duration`, and the DVB-TA compact `pts_time` / `duration` fields masked an out-of-range 33-bit or 40-bit value instead of erroring, unlike `SpliceInfoSection`'s own `pts_adjustment` check.

Documentation: the `dvb_ta::compact` module doc now carries an explicit warning that its reconstructed bit widths are unconfirmed. No real DVB TA compact-watermark capture or authoritative erratum has been obtained, so a wrong width would misdecode silently rather than error. Functionally unchanged (#1102 W5).

---

Published from tag `scte35-splice-v3.0.0`.
