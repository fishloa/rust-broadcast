# mpeg-ps 0.5.0

_Released 2026-10-05._

**Minor-epoch breaking release (0.x), and the first release where `PackHeader` and `ProgramStreamMap` match the spec's wire layout.** Three real-capture parse defects are fixed (`program_mux_rate` read about 5.2 times too small, PSM `elementary_stream_map_length` read from the wrong offset, `video_bound` masked to 4 bits), so values you read from the same bytes will change. Two structs gain public fields, serializers reject out-of-range input instead of masking it, and MPEG-1 packs are now reported distinctly. Act if you construct `Pack`/`SystemHeader` literals, read `program_mux_rate`, or iterate `pes_packets` expecting to see Program Stream Map packets. Read with [mpeg-pes 0.5.0](mpeg-pes-0.5.0.md) (dependency) and [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-mpeg-pes         = { version = "0.4", default-features = false }
+mpeg-pes         = { version = "0.5", default-features = false }
```

The package description changed from "MPEG-1/2 Program Stream parser" to "MPEG-2 Program Stream parser", matching the corrected crate-root docs (see MPEG-1 below). `PACKET_START_CODE_PREFIX` is now re-exported from `mpeg-pes` at the same path and value (#1141).

## Breaking changes

1. `Pack` gains `psm: Option<ProgramStreamMap<'a>>`. A Program Stream Map packet (`stream_id 0xBC`) is now parsed with `ProgramStreamMap::parse` and surfaced here; before, it was passed to `mpeg_pes::PesPacket::parse` and returned as an opaque entry in `pes_packets`. Code that looked for the PSM in `pes_packets` must read `pack.psm` (#1119).
2. `SystemHeader` gains `reserved_bits: u8` (the 7 reserved bits after `packet_rate_restriction_flag`, Table 2-40), preserved on parse and written back instead of a hard-coded `0x7F`, so such a header round-trips byte-identically. Struct-literal construction needs the new field (conventional value `0x7F`) (audit r14-MPS-W8, #1119).
3. `PackHeader::serialize_into` and `ProgramStreamMap::serialize_into` return `Err` for out-of-range fields (`stuffing_length` over 7, `program_mux_rate` over 22 bits, `reserved` over 5 bits, PSM `version` over 5 bits) instead of masking them, which framed `serialized_len()` and the written bytes inconsistently (#1129). `PackHeader::serialize_into` now writes `self.stuffing` verbatim (after checking it agrees with `stuffing_length`) instead of always `0xFF`, so a non-`0xFF` stuffed input round-trips (#1119 W8). A mismatch is the new `Error::StuffingLengthMismatch`.
4. `program_stream::parse_pack` / `parse_all_packs` no longer pre-scan the pack buffer for the next `pack_start_code`/`program_end_code` (`find_next_boundary` is removed). A boundary is only checked right after a whole PES packet, so a stray `000001BA`/`000001B9` inside a real PES payload (possible in AC-3, LPCM and DVD-subpicture `private_stream_1`) no longer truncates the pack (#1119 W3). The resynchronise-after-corruption role moves to the new `scan_packs` below.
5. New `Error` variants `Mpeg1NotSupported`, `StuffingLengthMismatch` and `FieldOverflow` (`Error` is `#[non_exhaustive]`, so a wildcard arm already covers them).

```rust
// before
let h = SystemHeader { /* ... */ std_buffer_bounds, ..  };
// after: also set the reserved bits
let h = SystemHeader { /* ... */ reserved_bits: 0x7F, std_buffer_bounds, .. };
```

## New

`program_stream::scan_packs(&[u8]) -> PackScan` walks a stream like `parse_all_packs` but, after a pack that fails to parse, records the failed span and error in `PackScan::skipped` (`SkippedRegion { offset, len, error }`) and resynchronises on the next `pack_start_code` instead of abandoning the stream. `PackScan::packs` holds every pack that parsed and `remaining` the trailing bytes. The byte search is confined to this recovery path; well-formed streams are still walked by `PES_packet_length` (audit r14-MPS-W3, #1119).

## Fixes that change values you read

- `PackHeader::program_mux_rate` (Table 2-39): parse and serialize assumed a `'01'` marker prefix before the 22-bit field (the SCR layout) that Table 2-39 does not have. Every real pack header's rate was read about 5.2 times too small, with the two trailing marker bits folded into the value, and every serialized rate failed re-parse (#1049).
- `ProgramStreamMap`: `elementary_stream_map_length` was read from, and written at, a fixed offset right after `program_stream_info_length`; Table 2-41 puts it after the program descriptor loop. Any conformant PSM with a non-empty program descriptor loop was misparsed, and every PSM this crate serialized with program descriptors was non-conformant (#1050).
- `SystemHeader::parse` masked `video_bound` to 4 bits instead of 5, so `video_bound` in `16..=31` round-tripped as `video_bound - 16` (#1119 W1).
- `parse_pack` positioned PES data at the system header's re-serialized length instead of its wire `header_length`, so a conformant header declaring trailing reserved padding made PES parsing start inside the system header and fail (#1119 W2).
- A pack header with the ISO/IEC 11172-1 (MPEG-1) `'0010'` prefix was reported as `BadScrPrefix`, the same as corruption. It is now `Error::Mpeg1NotSupported`; MPEG-1 packs remain unsupported and the crate-root doc is corrected to MPEG-2-only (#1119 W4).
- `ProgramStreamMap::serialize_into` wrote the reserved bit and the reserved+marker byte as `0` and `0x7F` instead of the conventional `1`s/`0xFF`, so a real capture with them set did not round-trip (#1119 W8).

## Fixes in length handling (#1129)

`pseudo_descriptor_length` was written as `1 + descriptors.len() as u8` (the cast preceded the add, so a 255-byte descriptor overflowed); `program_stream_map_length` (spec maximum 1018), `program_stream_info_length`, `elementary_stream_map_length` and `ES_info_length` used unchecked `as u16`; and `SystemHeader`'s stream-loop length accumulated in `u16`, overflowing (debug panic, out-of-bounds write in release) beyond 10,921 caller-constructed bounds. All are now range-checked, and the accumulator is `usize` with a separate check against the 16-bit `header_length` field.

## Internal

`ProgramStreamMap::serialize_into` now writes straight into the output instead of via per-entry copies (the `OwnedEsMapEntry` type and dead constants are gone, layout literals are named) (audit r14-MPS-O1), and scratch comments in `SystemHeader::parse` were removed.

---

Published from tag `mpeg-ps-v0.5.0`.
