# Changelog

## Unreleased

### Added
- `PesHeader::header_stuffing_len: usize` — number of trailing `0xFF` stuffing
  bytes inside the `PES_header_data_length` region, after the typed optional
  fields (ISO/IEC 13818-1 §2.4.3.7). Captured on parse and re-emitted on
  serialize so a stuffed PES header round-trips **byte-identical**. Construct
  with `header_stuffing_len: 0` for no stuffing.
- `PesHeader` is now `#[non_exhaustive]` (matching the workspace convention), so
  future optional fields are additive.
- `Escr` — 33-bit base (90 kHz) + 9-bit extension (27 MHz) ESCR type with
  `from_27mhz`, `as_27mhz`, `from_field_bytes`, `to_field_bytes`
  (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
- `TrickMode` — fully typed enum for `DSM_trick_mode_flag` (§2.4.3.8, Table 2-24):
  `FastForward`, `SlowMotion`, `FreezeFrame`, `FastReverse`, `SlowReverse`,
  `Reserved`; `from_byte` / `to_byte`.
- `PesExtension` — typed `PES_extension_flag` sub-structure with
  `pes_private_data` (16 bytes), `pack_header`, `program_packet_sequence_counter`
  (`ProgramPacketSequenceCounter`), `p_std_buffer` (`PStdBuffer`), and
  `pes_extension_field`.
- `PesHeader` — replaced the raw `optional_fields: &[u8]` blob with fully typed
  fields: `escr`, `es_rate` (22-bit), `dsm_trick_mode`, `additional_copy_info`
  (7-bit), `pes_crc`, `pes_extension`. PTS/DTS retained.
- `Pts::from_field_bytes`, `Pts::from_field_bytes_with_dts`, `Dts::from_field_bytes`
  — parse direction for the PTS/DTS 5-byte wire fields.
- All new types are symmetric: `parse` + `serialize_into` with round-trip tests.

### Changed
- `PesHeader::optional_fields` removed — replaced by fully typed sub-fields
  (`escr`, `es_rate`, `dsm_trick_mode`, `additional_copy_info`, `pes_crc`,
  `pes_extension`). Breaking change; struct is now `#[non_exhaustive]`.

### Fixed
- `PesPacket::serialize_into` now reproduces the `PES_header_data_length`
  `0xFF` stuffing instead of dropping it (and writes the original
  `PES_header_data_length`), so parse → serialize is byte-identical for real
  broadcast PES headers. Verified on a `test-001.ts`-derived fixture (PTS+DTS and
  PTS-only headers, each with stuffing) round-tripping byte-for-byte.

## [0.1.3] — 2026-06-29

### Added
- `Escr` — 33-bit base (90 kHz) + 9-bit extension (27 MHz) ESCR type with
  `from_27mhz`, `as_27mhz`, `from_field_bytes`, `to_field_bytes`
  (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
- `TrickMode` — fully typed enum for `DSM_trick_mode_flag` (§2.4.3.8, Table 2-24):
  `FastForward`, `SlowMotion`, `FreezeFrame`, `FastReverse`, `SlowReverse`,
  `Reserved`; `from_byte` / `to_byte`.
- `PesExtension` — typed `PES_extension_flag` sub-structure with
  `pes_private_data` (16 bytes), `pack_header`, `program_packet_sequence_counter`
  (`ProgramPacketSequenceCounter`), `p_std_buffer` (`PStdBuffer`), and
  `pes_extension_field`.
- `PesHeader` — replaced the raw `optional_fields: &[u8]` blob with fully typed
  fields: `escr`, `es_rate` (22-bit), `dsm_trick_mode`, `additional_copy_info`
  (7-bit), `pes_crc`, `pes_extension`. PTS/DTS retained.
- `Pts::from_field_bytes`, `Pts::from_field_bytes_with_dts`, `Dts::from_field_bytes`
  — parse direction for the PTS/DTS 5-byte wire fields.
- All new types are symmetric: `parse` + `serialize_into` with round-trip tests.

### Changed
- `PesHeader::optional_fields` removed — replaced by fully typed sub-fields
  (`escr`, `es_rate`, `dsm_trick_mode`, `additional_copy_info`, `pes_crc`,
  `pes_extension`). Breaking change; struct is now `#[non_exhaustive]`.

## [0.1.2] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.1→mpeg-pes] Crate renamed from `dvb-pes` to `mpeg-pes`; code unchanged.

## 0.1.1 — 2026-06-19

### Added
- `examples/`: `parse_pes_packet` (parse one PES packet from bytes) and
  `extract_pts` (depacketize a capture, reassemble PES, report the PTS timeline).

## 0.1.0 — 2026-06-18

### Added
- Initial release. PES (Packetized Elementary Stream) depacketization per
  ISO/IEC 13818-1 (Rec. ITU-T H.222.0) §2.4.3.6 / §2.4.3.7:
  - `PesPacket` / `PesHeader` — PES packet header parse + symmetric serialize,
    incl. unbounded video (`PES_packet_length = 0`) and the special `stream_id`s
    that carry no optional header.
  - `Pts` / `Dts` — 33-bit, 90 kHz timestamps with marker-bit-aware decode/encode.
  - `StreamId` — `stream_id` newtype with `has_optional_header()` / `is_audio()` /
    `is_video()`.
  - `PesAssembler` — per-PID PES reassembly from TS payloads, PUSI-driven.
  - `#![no_std]` + `alloc`; depends only on `dvb-common`; `serde` feature.
