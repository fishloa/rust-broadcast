# mpeg-ps 0.9.0

_Released 2026-11-15._

### Changed
- BREAKING: `ProgramStreamMap::program_stream_info` and `EsMapEntry::descriptors` are `DescriptorLoop` (new module `mpeg_ps::descriptor_loop`) instead of `&[u8]`. The loop wraps the wire bytes verbatim (`raw()`), walks the TLV structure with `entries()` (`DescriptorEntry { tag, body }`, a truncated entry reported as `Error::DescriptorHeaderTruncated`/`DescriptorBodyTruncated` rather than silently dropped), and decodes every entry through the caller's own parser with `parse_with` (mpeg-ps keeps no `dvb-si` dependency: `dvb-si` is above `mpeg-ts`, which is above this crate, so the caller names its decoder). A caller no longer hand-builds the `descriptor()` bytes to read them. Wire bytes are unchanged. Migration: `&psm.program_stream_info[..]` -> `psm.program_stream_info.raw()`, `EsMapEntry { descriptors: &bytes, .. }` -> `EsMapEntry { descriptors: DescriptorLoop::new(&bytes), .. }` (#2770).
- BREAKING: `program_stream_map::PACKET_START_CODE_PREFIX` (the module-local `u32` form) is private; the single public definition is `mpeg_ps::PACKET_START_CODE_PREFIX` (the `[u8; 3]` re-exported from `mpeg-pes`). The two used to be two different types under one name (#2774).
### Fixed
- The pack walker advances past a Program Stream Map by its wire length (`6 + program_stream_map_length`, ISO/IEC 13818-1 Table 2-41) instead of the re-serialized length of the parsed struct, which is shorter whenever the ES loop carries bytes no entry claims; every later pack of such a stream used to be lost (#2743).
- BREAKING: `ProgramStreamMap::parse` rejects a PSM whose elementary-stream entries do not cover the declared `elementary_stream_map_length` with the new `Error::ElementaryStreamMapTrailingBytes { declared, parsed }`, instead of silently dropping the remainder (#2743).
- `Pack` implements `Serialize` (the pack header, then the system header, the PSM and the PES packets `parse_pack` accepts), so every parsed pack round-trips byte-exactly (#2744).
### Changed
- `PackHeader::serialize_into` rejects a zero `program_mux_rate` with `Error::ZeroMuxRate`, which `parse` already refuses, instead of writing a header that cannot be read back (#2744).
- `ProgramStreamMap::parse` enforces the 1018-byte `program_stream_map_length` ceiling the serializer already enforces, so every accepted PSM can be written back (#2744).
- BREAKING: `EsMapEntry` gains `pseudo_descriptor_tag` and `pseudo_descriptor_length` (both `Option<u8>`; `None` means "write the spec value" `0x01`/1). The parser keeps the two fields instead of discarding them and the serializer writes them back, so a non-conforming stream round-trips; a rebuilt entry whose descriptors do not cover a non-spec `pseudo_descriptor_length` is refused rather than misframed (#2744). The spec values are ISO/IEC 13818-1 §2.5.4.2.
- BREAKING: `program_stream::parse_all_packs` is removed; `program_stream::scan_packs` is the one walker (it keeps every pack that parsed and reports each skipped span with its error, where `parse_all_packs` discarded every good pack on the first failure). `pack_offsets`-style strict walking stays available as `parse_pack` in a loop (#2745).
- BREAKING: `Pack`, `PackScan` and `SkippedRegion` are `#[non_exhaustive]` (#2745).
- The `serde` feature enables `broadcast-common/serde`, so the clock value the SCR is built on serializes.
### Added
- `Scr::from_clock27(Clock27)` (construct from the validated clock value) and `Scr::to_clock27()` (the checked view). Parsing stays lossless: a real pack header whose 9-bit SCR extension is 300..=511 keeps the parts as read and re-serializes byte-identically, and `to_clock27` reports it as `ClockOutOfRange` rather than making the pack header unparseable (#2656).

---

Published from tag `mpeg-ps-v0.9.0`.
