# mpeg-ps 0.9.0

_Released 2026-11-15._

Minor-class release (0.8 -> 0.9). Two long-standing stream walker bugs are fixed (a Program Stream Map advanced by the wrong length lost every later pack, and a PSM that did not cover its declared map length was accepted with the remainder dropped), and a pack header, system header, PSM and PES packet now round-trip byte-exactly. The descriptor loops are typed and the pack scan is the single walker.

Version: 0.8.0 -> 0.9.0.

## Breaking changes and migration

### Program Stream Map descriptor loops are `DescriptorLoop`

`ProgramStreamMap::program_stream_info` and `EsMapEntry::descriptors` are `DescriptorLoop` (new module `mpeg_ps::descriptor_loop`) instead of `&[u8]`. The loop wraps the wire bytes verbatim (`raw()`), walks the TLV structure with `entries()` (`DescriptorEntry { tag, body }`, with a truncated entry reported as `Error::DescriptorHeaderTruncated`/`DescriptorBodyTruncated` rather than silently dropped), and decodes every entry through the caller's own parser with `parse_with` (mpeg-ps keeps no `dvb-si` dependency, so the caller names its decoder). Wire bytes are unchanged.

```rust
// before
for byte in psm.program_stream_info { .. }
let entry = EsMapEntry { descriptors: &bytes, .. };
// after
for entry in psm.program_stream_info.entries() { let entry = entry?; let (tag, body) = (entry.tag, entry.body); }
let tags: Vec<u8> = psm.program_stream_info.parse_with(|tag, _body| tag).collect::<Result<_, _>>()?;
let entry = EsMapEntry { descriptors: DescriptorLoop::new(&bytes), .. };
```

### `program_stream::parse_all_packs` is `scan_packs`

`parse_all_packs` discarded every good pack on the first failure; `program_stream::scan_packs` keeps every pack that parsed and reports each skipped span with its error. A strict walk stays available as `parse_pack` in a loop.

```rust
// before
let packs = program_stream::parse_all_packs(&bytes)?;
// after
let scan = program_stream::scan_packs(&bytes);
for pack in &scan.packs { /* every pack that parsed, in stream order */ }
for region in &scan.skipped { let error = &region.error; /* each skipped span */ }
```

### `EsMapEntry` keeps its pseudo-descriptor fields

`EsMapEntry` gains `pseudo_descriptor_tag` and `pseudo_descriptor_length` (both `Option<u8>`; `None` means "write the spec value" `0x01`/1). The parser keeps the two fields instead of discarding them and the serializer writes them back, so a non-conforming stream round-trips; a rebuilt entry whose descriptors do not cover a non-spec `pseudo_descriptor_length` is refused rather than misframed.

```rust
// before
let entry = EsMapEntry { stream_type, elementary_stream_id, descriptors: DescriptorLoop::new(&bytes) };
// after
let entry = EsMapEntry {
    stream_type,
    elementary_stream_id,
    descriptors: DescriptorLoop::new(&bytes),
    stream_id_extension: None,
    pseudo_descriptor_tag: None,        // write the spec 0x01
    pseudo_descriptor_length: None,     // write the spec 1
};
```

### `Pack`, `PackScan` and `SkippedRegion` are `#[non_exhaustive]`

Read them through their accessors instead of building them by struct literal.

```rust
// before
let first = scan.packs[0];
if scan.packs.is_empty() { /* .. */ }
// after
match program_stream::scan_packs(&bytes).packs.first() {
    Some(first) => { /* .. */ }
    None => { /* nothing parsed */ }
}
```

### `PACKET_START_CODE_PREFIX` has one definition

`program_stream_map::PACKET_START_CODE_PREFIX` (a `u32`) is private; the public definition is `mpeg_ps::PACKET_START_CODE_PREFIX`, the `[u8; 3]` re-exported from `mpeg-pes`. The two used to be two different types under one name.

```rust
// before
use mpeg_ps::program_stream_map::PACKET_START_CODE_PREFIX;
// after
use mpeg_ps::PACKET_START_CODE_PREFIX;   // [u8; 3]
```

## Fixes

- The pack walker advances past a Program Stream Map by its wire length (`6 + program_stream_map_length`, ISO/IEC 13818-1 Table 2-41) instead of the re-serialized length of the parsed struct, which is shorter whenever the ES loop carries bytes no entry claims; every later pack of such a stream used to be lost.
- `ProgramStreamMap::parse` rejects a PSM whose elementary-stream entries do not cover the declared `elementary_stream_map_length` with `Error::ElementaryStreamMapTrailingBytes { declared, parsed }` instead of silently dropping the remainder.
- `PackHeader::serialize_into` rejects a zero `program_mux_rate` with `Error::ZeroMuxRate`, which `parse` already refuses, instead of writing a header that cannot be read back.
- `ProgramStreamMap::parse` enforces the 1018-byte `program_stream_map_length` ceiling the serializer already enforces, so every accepted PSM can be written back.

## Added

- `Pack` implements `Serialize` (the pack header, then the system header, the PSM and the PES packets `parse_pack` accepts), so every parsed pack round-trips byte-exactly.
- `Scr::from_clock27(Clock27)` and `Scr::to_clock27()`. Parsing stays lossless: a pack header whose 9-bit SCR extension is 300..=511 keeps the parts as read and re-serializes byte-identically, and `to_clock27` reports it as `ClockOutOfRange` rather than making the pack header unparseable.
- The `serde` feature enables `broadcast-common/serde`, so the clock value the SCR is built on serializes.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `mpeg-pes` 0.8 -> 0.9.

---

Published from tag `mpeg-ps-v0.9.0`.
