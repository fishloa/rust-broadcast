# mpeg-pes 0.5.0

_Released 2026-10-05._

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

---

Published from tag `mpeg-pes-v0.5.0`.
