# Changelog

All notable changes to `smpte2038` will be documented in this file.
This crate was previously published as `dvb-smpte2038` (0.1.0). The rename
reflects that SMPTE ST 2038 is a SMPTE standard, not a DVB standard.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-07-03
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
