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

## [0.2.0] - 2026-07-11

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
  - `st291/docs/anc_rtp_8331.md` — the RFC 8331 curation for the
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

## [0.1.0] - 2026-07-11

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
