# Changelog

All notable changes to `st291` will be documented in this file.
This crate was previously published as `smpte2038` (0.1.0-0.2.0), and before
that as `dvb-smpte2038` (0.1.0). The rename reflects that the crate's real
subject is ST 291-1 ancillary-data *content*, not any one carriage mechanism:
ST 2038 (MPEG-2 TS/PES) is the first of what will become multiple transports
once RTP carriage (ST 2110-40 / RFC 8331, issue #648) is added. Both
`smpte2038` and `dvb-smpte2038` are being yanked from crates.io rather than
carried forward as deprecated shims — this is a clean break, not a shim
chain. `st291` restarts version numbering at 0.1.0.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- The wire-format contract types now come from `broadcast-core`.

## [0.9.0] - 2026-11-22
### Breaking

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mpeg-pes` 0.9 -> 0.10, `rtp-packet` 0.8 -> 0.9). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever its own API diff, so this release is a major-class bump; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the PES/RTP types it takes are those versions' types.

### Fixed
- `Error::ReservedBits` for a cleared `data_alignment_indicator` reported `expected: 0`, the position bit masked by `& 1`, instead of the spec value `1` (#3210).

### Tests
- Pin `Checksum_Word` bit 9 (the inverse of bit 8) with a `b8 = 0` literal and the fixture's on-wire words, and the `AncPacket::checksum()` accessor (#3209).
- Reject each forbidden ST 2038 PES header field on its own, so the `||` chain is exercised one condition at a time (#3210).
- Pin `PLACEMENT_BITS == 32`, the record bit/byte lengths, the exact 65535 `PES_packet_length` ceiling, `Descriptor` labels and the per-position reserved-field check (#3211).

## [0.8.0] - 2026-11-15
### Changed
- BREAKING: `AncRtpPayload::extended_sequence_number` is an `ExtendedSequenceNumber` instead of a bare `u16` (#2768): the field holds the high-order bits of RFC 4175's extended 32-bit sequence number, so it now names that position (`high_half()`, `extended(rtp_sequence_number)`) rather than leaving the shift to the caller. The two wire bytes are unchanged.
- BREAKING: `AncDataDescriptor::inner_descriptors: &[u8]` is now `descriptors: Vec<Descriptor>` (#2770). The `descriptor_length` body is an ISO/IEC 13818-1 §2.6 `descriptor()` loop, so each entry is walked into a typed (borrowed) `Descriptor` with `tag()`/`body()` accessors instead of the caller re-parsing raw octets. ST 2038 §4.1.2.1 defines no descriptors for the loop, so every entry is `Descriptor::Other { tag, body }`, kept verbatim for a lossless round-trip; a truncated entry header or body is now reported as `Error::BufferTooShort` instead of being retained as opaque bytes.
- The `panic_idioms` workspace guard now covers this crate; the `#[cfg(all(test, any(feature = "ts", feature = "rtp")))]` test module is gated as stacked `#[cfg(test)]` + `#[cfg(any(feature = "ts", feature = "rtp"))]` so the shared lexer recognises it as test-only (#2773).
### Fixed
- `AncPacket` can no longer be serialized with a stale `Data_Count` or `Checksum_Word` (#2762): ST 291-1:2011 §6.5/§6.7 derive both from `DID`/`SDID`/`User_Data_Words`, so they are now computed on serialize instead of written from a stored field. Editing a user data word used to write bytes whose declared count/checksum contradicted the words themselves; the round-trip test `hand_computed_wire_bytes` still pins the canonical bytes (derived `Data_Count` `0x102`, checksum `0x134`).
- The ANC PES header is parsed and written by `mpeg-pes`'s `PesPacket` (`Error::Pes` wraps its errors) instead of a second copy of the PES timestamp/BER coding (#2762); ST 2038 Table 2's pinned fields (`stream_id` `0xBD`, `PES_scrambling_control` `'00'`, `data_alignment_indicator` `'1'`, PTS-only, `PES_header_data_length` `0x05`) are still enforced on top. The `.expect("length checked above")` in the hand-rolled PTS reader is gone with it.

### Breaking
- `AncPacket` no longer has `data_count` or `checksum` fields; `data_count()`, `checksum()` and `udw_loop_count()` are fallible methods (`Error::FieldOverflow` past 255 user data words), and `build(c_not_y_channel_flag, line_number, horizontal_offset, did8, sdid8, udw8s)` builds a packet with parity filled in (#2762).
- `AncPacket::validate` returns `Result<ValidationReport>` so its derived-field check can report a length overflow.
- Parsing rejects a packet whose stored `Data_Count` or `Checksum_Word` disagrees with its fields (new `Error::DataCountMismatch`; existing `Error::ChecksumMismatch`) instead of silently rewriting them on serialization.
- `Error::Pes(mpeg_pes::Error)` replaces the PES-header error variants `BadPtsDtsFlags` and `BadHeaderDataLength`; a malformed PES header (bad start code, bad `PES_header_data_length`, a payload shorter than `PES_packet_length`) is now `Error::Pes(..)`, the last case as `mpeg_pes`'s own `BufferTooShort` (#2746). `Error::PesLengthOverflow` is removed (the check moved into `mpeg_pes`); `Error::BadStreamId` and `Error::ReservedBits` stay.
- PTS construction goes through `mpeg_pes::Pts::try_new` and `PES_scrambling_control` is checked against `mpeg_pes::ScramblingControl::NotScrambled` (matching mpeg-pes's checked-timestamp and typed-scrambling fields, #2747); the `Error::FieldOverflow { field: "PTS" }` an over-range PTS reports is unchanged.

### Dependencies
- New dependency on the workspace sibling `mpeg-pes` (the PES header codec, #2762). No published version requires a caret-epoch change.

## [0.7.0] - 2026-11-14
### Changed
- Major-class release for epoch purity only: no API or behaviour change in this crate; its sibling dependencies move to new caret epochs.

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `rtp-packet` 0.6 -> 0.7.

## [0.6.0] - 2026-11-07
### Fixed
- Serializers report a too-small output buffer as the new `Error::OutputBufferTooSmall { need, have }` (with `From<broadcast_common::len::OutputBufferTooSmall>`) instead of the input-side `Error::BufferTooShort` (c6 C6-02).
### Changed
- Length pre-checks go through `broadcast_common::len::{need_in, need_out}`; the error values are unchanged (c3 S1).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
- BREAKING: error variants `BufferTooShort`, `OutputBufferTooSmall`, `LengthMismatch` now wrap `broadcast_common::error` types (tuple variants; `LengthMismatch` gains `what`, `computed` is now `actual`); `BadStartCode(u32)` is replaced by `InvalidSync`, `BadFixedBits(&str)` and `ReservedNotZero { what, value }` by one `ReservedBits(broadcast_common::error::ReservedBits)` carrying the expected and found values, `FieldTooWide { what, value: u32, bits }` by `FieldOverflow(broadcast_common::error::FieldOverflow)` (values are no longer truncated to `u32`). Hand-written `From` impls removed.
### Added
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type, producing the identical existing variants (c3 S1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `rtp-packet` 0.5 -> 0.6. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.5.0] - 2026-10-17
### Added
- Explicit ST 291-1:2011 parity validation, grounded in the vendored standard itself (sections 6.1, 6.2, 6.4, 6.5, 6.7; verified on rendered PDF pp. 9, 12, 17): `AncContent::has_valid_parity`, `AncContent::verify_parity` (`Error::ParityMismatch`), `AncContent::validate` and `AncPacket::validate` returning a typed `ValidationReport` (DID/SDID/Data_Count parity, Data_Count vs UDW count, checksum). Opt-in only; parsing remains tolerant. Spec transcription in `private/specs-verified/smpte/st291/anc_parity_checksum.md`. Tests use Annex C's worked DID/DBN/DC words and hand-computed vectors with flipped-bit cases.
- Finding and fix: the committed `anc.bin`/`anc_rtp.bin` fixtures are synthetic (hand-constructed from ST 2038 Table 2, not real captures) and used to fail ST 291-1 (Data_Count had no parity bits, checksums off by one). They are regenerated with the crate's own `AncContent::with_parity`/`compute_checksum` (`Data_Count` `0x102`/`0x203`, checksums `0x134`/`0x1AC`; all other bytes unchanged) and `tests/fixture_validate.rs` now asserts `is_valid()` on both, with a separate hand-built non-conformant packet providing the bite (it previously pinned the failures, so it could not catch a real defect).

### Documentation
- README and crate-root docs restructured: what it does / does not, standards-supported table (ST 2038:2021, RFC 8331, ST 2110-40, ST 291-1), feature table, design notes; removed the stale "parity/checksum not validated" claim (explicit `verify_checksum`/`build` helpers exist).

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `rtp-packet` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

## [0.4.1] - 2026-10-05
### Fixed
- The ST 2038 PES serializer no longer silently masks a PTS of 2^33 or more
  to a different (wrapped) timestamp; it now rejects it with
  `Error::FieldTooWide` (#1116, #1129).
- `AncDataPacket::parse` (through the private `AncPacket::read_from`) now rejects a
  nonzero leading `'000000'` field, a byte-alignment padding region that is
  not all `'1'` bits, and nonzero `ESCR_flag`/`ES_rate_flag`/
  `DSM_trick_mode_flag`/`additional_copy_info_flag`/`PES_CRC_flag`/
  `PES_extension_flag` bits in the PES optional header — all three were
  previously skipped unchecked, so a corrupt packet parsed `Ok` and
  reserialized to different bytes (#1116). The error is
  `Error::BadFixedBits`.
- Named the byte-6/byte-7 PES flag bitmasks instead of inline hex literals
  (#1116).

### Added
- `AncContent::with_parity`/`compute_checksum`/`verify_checksum`/`build`:
  compute and verify the ST 291-1 parity bits and `Checksum_Word` that
  parsing/serializing never validated (#1116). `Error::ChecksumMismatch` is
  new.

## [0.4.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.2.0] - 2026-07-11

Bundles the `st291` rename (0.1.0 was never separately published under this
name — see below).

### Added
- A new `rtp` feature: RFC 8331 / ST 2110-40 carriage of ST 291-1 ANC data
  packets over RTP (issue #648, epic #645 story 3), sitting alongside the
  existing `ts` feature.
  - `AncContent` — the shared ST 291-1 ANC-packet content
    (`DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word`), factored
    out of `AncPacket`'s existing bit-packing logic. Always compiled, gated
    behind neither `ts` nor `rtp`, so enabling one transport never pulls in
    the other; `AncPacket`'s own public field layout is unchanged.
  - `RtpAncPacket` — the RFC 8331 §2.1 per-ANC-packet placement
    (`C`/`Line_Number`/`Horizontal_Offset`/`S`/`StreamNum`) wrapping
    `AncContent`.
  - `AncRtpPayload` — the full RFC 8331 §2.1 payload (`Extended Sequence
    Number`/`Length`/`ANC_Count`/`F`/`reserved` + the `RtpAncPacket` list),
    implementing `broadcast_common::Parse`/`Serialize`. `Length` and
    `ANC_Count` are always recomputed on serialize and cross-validated
    against the wire value on parse (a corrupted `Length` or `ANC_Count` is
    rejected, never silently trusted); the 22-bit `reserved` field and each
    per-packet `word_align` padding are validated as zero.
  - `AncRtpPayload::parse_rtp_packet` — convenience composition riding on
    `rtp_packet::RtpPacket` (RFC 3550) for a full ANC-over-RTP packet.
  - `FieldSense` — the `F` field (2 bits): `ProgressiveOrUnspecified`/
    `Field1`/`Field2`, plus `Invalid` for the spec's `0b01` ("not valid")
    value — parses successfully rather than being rejected, per this
    project's decode-completeness principle (RFC 8331's "SHOULD ignore" is a
    receiver recommendation, not a parser-level rejection). `#[non_exhaustive]`,
    `name()` + `impl_spec_display!` (issue #204).
  - `private/specs-verified/smpte/st291/anc_rtp_8331.md` — the RFC 8331 curation for the
    RTP-transport-specific material (RTP-header semantics, the §2.1 payload
    header, §3.1/§4 media type + clock rate); reuses the already-audited
    `anc_packet_291.md` (per-ANC-packet fields + parity/checksum) unchanged.
  - Two new runnable examples (`build_anc_rtp`, `parse_anc_rtp`) and a
    fixture (`fixtures/st291/anc_rtp.bin`, built from the existing audited
    `anc.bin` content bytes wrapped in fresh RFC-8331 framing — see
    `fixtures/st291/anc_rtp-PROVENANCE.md`).
  - The libfuzzer fuzz target now also exercises `AncRtpPayload::parse` and
    `AncRtpPayload::parse_rtp_packet`.
- `Error::ReservedNotZero` and `Error::LengthMismatch` — new error variants
  for the `rtp` feature's payload-header/word-align validation.

### Changed
- Renamed `smpte2038` → `st291` (issue #647, part of epic #645). No functional
  or API change beyond the new `ts` feature below; every existing test still
  passes under the new name with identical behaviour.
- Added a `ts` feature (on by default) gating the existing ST 2038:2021
  MPEG-2 TS transport (`AncDataDescriptor`, `AncDataPacket`) — preparation for
  a future `rtp` feature (issue #648) so the two transports can sit side by
  side without another restructuring.
- Fixed a stale README line claiming the crate "depends only on `dvb-common`"
  (it depends on `broadcast-common`, predating this rename).

### Added
- A libfuzzer fuzz target (`fuzz/fuzz_targets/st291.rs`) exercising
  `AncDataPacket::parse` — previously absent for this crate's ANC parsing.

## [0.2.0] - 2026-07-03 (as `smpte2038`)
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.0] — 2026-06-21

### Fixed

- `AncPacket::write_into` (called from `AncDataPacket::serialize_into`) now
  returns `Error::InconsistentUdwLength { have, need }` when
  `user_data_words.len()` does not equal `data_count & 0xFF` (§4.2.1). The
  previous code silently zero-filled missing words, making serialize→parse
  non-identity on inconsistent structs.
- `AncDataPacket::parse` now rejects byte 6 values where
  `PES_scrambling_control != '00'` or `data_alignment_indicator != '1'` with
  `Error::BadFixedBits`, per SMPTE ST 2038:2021 Table 2 "shall" requirements.

### Added

- `Error::InconsistentUdwLength { have, need }` — new error variant for UDW
  length/`data_count` mismatch.
- `tests/label_coverage.rs` — drift-guard for the spec/field-enum label
  convention (issue #204); `Error` is in the skip list since it carries no spec
  label.


### Added

- `AncDataDescriptor` — parser+serializer for the `anc_data_descriptor`
  (SMPTE ST 2038:2021 §4.1.2, Table 1): tag `0xC4` + opaque inner descriptor
  loop. Plus the constants `ANC_STREAM_TYPE` (`0x06`, §4.1.1),
  `ANC_DATA_DESCRIPTOR_TAG` (`0xC4`), and `VANC_FORMAT_IDENTIFIER`
  (`0x56414E43`, the `"VANC"` `registration_descriptor` `format_identifier`,
  §4.1.3).
- `AncDataPacket` — parser+serializer for the ANC data PES packet
  (§4.2, Table 2): the fixed PES header (`stream_id == 0xBD`, PTS,
  `PES_header_data_length == 0x05`) + the contiguous MSB-first 10-bit
  bit-packed ANC-packet loop + trailing `0xFF` stuffing. `PES_packet_length`
  is recomputed on serialize.
- `AncPacket` — one ST 291-1 ANC data packet with its ST 2038 placement
  (`c_not_y_channel_flag`, `line_number`, `horizontal_offset`, 10-bit raw
  `DID`/`SDID`/`data_count`/`user_data_words`/`checksum`). The
  `user_data_word` loop counter uses only the **low 8 bits** of `data_count`
  (§4.2.1); the full 10-bit values are preserved verbatim. ST 291-1
  parity/checksum is not validated (deferred to ST 291-1, not vendored).
- Two runnable examples: `build_anc` (construct + serialize from typed fields)
  and `parse_anc` (parse the committed fixture at runtime + byte-exact
  round-trip).
- Real-ish fixture test (`tests/fixture_anc.rs`) over a committed 2-ANC-packet
  ANC data PES (`tests/fixtures/anc.bin`), validating decoded fields and a
  byte-exact round-trip.
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
