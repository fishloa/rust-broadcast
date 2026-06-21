# Changelog

All notable changes to `dvb-smpte2038` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

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
