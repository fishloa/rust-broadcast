# Changelog

## [Unreleased]

### Fixed
- Serialize: `PES_extension` `reserved_bits` was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

### Added (#1224)
- `StreamId::EXTENDED_STREAM_ID` (0xFD), `STREAM_ID_EXTENSION_AVS3_VIDEO` (0x41) and `STREAM_ID_EXTENSION_AVS3_AUDIO` (0b100_1111) (ETSI TS 101 154 V2.10.1 section 4.1.8; T/AI 109.7 clause 8.1.2), `PesExtensionField::stream_id_extension()` accessor, and `PesExtensionField` is now re-exported.

### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

### Fixed

- **`P-STD_buffer_size` over 13 bits is rejected (#1129)** instead of being masked with `& 0x1F`; `PesExtension::serialize_into` returns `Error::FieldOverflow`.

### Changed
- ESCR field encoding uses named marker/mask constants (no behaviour change; audit r01-W15 residue).

### Changed (breaking, umbrella #1139 T11 / audit r01-W2)
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

### Added
- `tests/pes_header_roundtrip_exhaustive.rs`: every PES packet reassembled from
  every committed `.ts` fixture re-serializes byte-exactly; real evidence for
  header stuffing (`pes-pts-dts-stuffing.ts`) and for `reserved == 0b000`
  (`dirac-vc2-extended-stream-id.ts`); spec-built vectors for non-`0xFF`
  stuffing and all 8 reserved values.

## [0.5.0] - 2026-10-05
### Changed (breaking)
- `PesExtension::pes_extension_field` is now `Option<PesExtensionField<'a>>`
  instead of `Option<&'a [u8]>` (#1052): `PesExtensionField` types the
  `stream_id_extension_flag`(1)+7-bit byte per ISO/IEC 13818-1 §2.4.3.7,
  Table 2-21, instead of leaving it as an opaque slice.

### Changed
- `PesAssembler` is now a thin wrapper over `broadcast_common::pusi::PusiAccumulator` (shared with `mpeg-ts`'s `PusiReassembler`; audit r01-W13, #1074). New `PesAssembler::with_max_unit_size`.
- Requires `broadcast-common` 9.4 (`broadcast_common::len`). A new
  `Error::FieldOverflow` variant is added.

### Fixed
- `PesAssembler` buffered a PES without bound when no further PUSI arrived (legal for `PES_packet_length == 0` video), so a hostile or broken stream could exhaust memory. Each PES is now capped at 16 MiB by default; an oversized PES is discarded and reassembly resumes at the next PUSI (audit r01-W13, #1074).
- `PesExtension`'s `PES_extension_field_length` was parsed and serialized as
  a plain 8-bit length; Table 2-21 has `marker_bit(1) +
  PES_extension_field_length(7)`, so every conformant
  `PES_extension_flag_2 == 1` packet (in practice, every `stream_id == 0xFD`
  "extended_stream_id" PES) failed to parse with `BufferTooShort` (#1052).
  Verified against an `ffmpeg` `vc2` (Dirac)-in-TS fixture, one of the few
  codecs that produces a real `0xFD` extended-stream-id PES — see
  `mpeg-pes/tests/fixture_pes_extension.rs`.
- `PesExtension::serialize_into` wrote `pack_field_length` and
  `PES_extension_field_length` with unchecked `as u8` casts, wrapping to a
  short length for data over 255 bytes while the full data was still copied
  after it (#1129). Unreachable via the public `PesPacket` API today (a
  pre-existing `PesHeader::optional_len` check already rejects the same
  oversized case first), fixed as defense-in-depth for the same reason
  `scte35-splice`'s `write_header` was.

## [0.4.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Changed (Breaking)
- `TrickMode` (`packet`) now carries `#[non_exhaustive]` (issue #806's
  non_exhaustive drift-guard audit: every other public enum in the workspace
  already did). A downstream `match` on `TrickMode` now needs a wildcard arm.

### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806).

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.3] — 2026-07-01
### Added
- `examples/build_pes_header.rs` — walk-through demonstrating the build/edit
  API: construct a PES header from typed fields (PTS+DTS, ESCR, DSM trick
  mode), serialize, re-parse, and verify round-trip.
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

## [0.1.1] — 2026-06-19

Crate renamed from `dvb-pes` to `mpeg-pes` at this version; code unchanged.
Also published under the old name as `dvb-pes` 0.1.1.

### Added
- `examples/`: `parse_pes_packet` (parse one PES packet from bytes) and
  `extract_pts` (depacketize a capture, reassemble PES, report the PTS timeline).

## [0.1.0] — 2026-06-18 (as `dvb-pes`; never published as `mpeg-pes`)

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
