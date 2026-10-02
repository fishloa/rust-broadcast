# Changelog

All notable changes to `mpeg-ps` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Requires `broadcast-common` 9.4 (`broadcast_common::len`). A new
  `Error::FieldOverflow` variant is added.

### Added
- `program_stream::scan_packs` + `PackScan`/`SkippedRegion`: walks a Program
  Stream like `parse_all_packs` but, after a pack that fails to parse,
  records the failed span and error and resynchronises on the next
  `pack_start_code` instead of abandoning the stream (audit r14-MPS-W3, #1119).
  The byte search is confined to this recovery path; well-formed streams are
  still walked by `PES_packet_length`.

### Changed (breaking)
- `SystemHeader` gains `reserved_bits: u8` (the 7 `reserved_bits` after
  `packet_rate_restriction_flag`, Table 2-40), preserved on parse and written
  back instead of a hard-coded `0x7F`, so a header with other reserved bits
  round-trips byte-identically (audit r14-MPS-W8, #1119). Trailing reserved
  bytes after the stream-bound loop (declared by `header_length`) are still
  skipped by the pack walker but not carried on `SystemHeader`.
- `Pack` gains a new field, `psm: Option<ProgramStreamMap<'a>>`. A Program
  Stream Map packet (`stream_id 0xBC`) is now parsed via
  `ProgramStreamMap::parse` and surfaced here instead of being handed to
  `mpeg_pes::PesPacket::parse` and returned as an opaque, un-mapped PES
  packet in `pes_packets` (#1119).
- `program_stream::parse_pack`/`parse_all_packs` no longer pre-scan the
  whole pack buffer for the next `pack_start_code`/`program_end_code`
  (`find_next_boundary` is removed); a boundary is now only ever checked
  right after a previously-consumed *whole* PES packet. A stray
  `000001BA`/`000001B9` inside a real PES payload (start-code emulation, not
  free of it in AC-3/LPCM/DVD-subpicture `private_stream_1` streams) no
  longer truncates the pack early (#1119 W3). Resync-on-parse-error after a
  genuinely corrupt PES packet is provided separately by the new
  `program_stream::scan_packs` (see Added).
- New `Error::Mpeg1NotSupported` and `Error::StuffingLengthMismatch`
  variants (both non-breaking on their own since `Error` is
  `#[non_exhaustive]`, listed here alongside the `Pack` field change).

### Fixed
- `ProgramStreamMap::serialize_into` wrote the elementary-stream loop through
  a per-entry owned copy of every descriptor slice and a second intermediate
  buffer; it now writes straight into the output, and the dead
  `PSM_START_CODE`/`HEADER_LEN` `#[allow(dead_code)]` constants and the
  `OwnedEsMapEntry` type are gone; layout literals are named constants
  (audit r14-MPS-O1, #1119).
- Removed stream-of-consciousness scratch comments from
  `SystemHeader::parse` and corrected two wrong bit-width notes in them.
- `SystemHeader::parse` masked `video_bound` (byte 4 bits `[4:0]`, Table
  2-40) to 4 bits (`& 0x0F`) instead of 5 (`& 0x1F`); serialize already used
  the correct mask, so `video_bound` in `16..=31` round-tripped to
  `video_bound - 16` (#1119 W1).
- `program_stream::parse_pack` positioned PES data at the system header's
  *re-serialized* length instead of its wire `header_length` (Table 2-40);
  a conformant `header_length` declaring trailing reserved padding past the
  stream-bound loop made PES parsing start early, inside the system header,
  and fail (#1119 W2).
- A pack header carrying the ISO/IEC 11172-1 (MPEG-1) `'0010'` pack prefix
  (a different, unimplemented 12-byte layout) was reported with the same
  `BadScrPrefix` as genuine corruption, despite this crate's docs claiming
  "MPEG-1/2" support. Now reported distinctly as `Error::Mpeg1NotSupported`,
  and the crate-root doc corrected to MPEG-2-only (#1119 W4).
- `ProgramStreamMap::serialize_into` wrote the flags byte's reserved bit
  (bit 5) as `0` and the following reserved+marker byte as `0x7F | 0x01`
  (`0x7F`, a no-op — `0x01` was already set) instead of the conventional
  `1`s/`0xFF`; a real capture with these bits set did not round-trip
  byte-identically (#1119 W8).
- `PackHeader::serialize_into` always wrote stuffing bytes as `0xFF`, even
  though `parse` already captures the actual bytes present in
  `self.stuffing` (`stuffing_byte` is spec-fixed `0xFF`, Table 2-39, but
  `parse` does not reject a non-conformant value) — a non-`0xFF` stuffed
  input did not round-trip byte-identically. Now writes `self.stuffing`
  verbatim, validating it agrees with `stuffing_length` first (#1119 W8).
- `PackHeader` `program_mux_rate` (Table 2-39): parse/serialize assumed a `'01'` marker prefix
  before the 22-bit field, the same layout the SCR field uses — Table 2-39 has no such prefix, so
  every real pack header's `program_mux_rate` was read 5.2x too small (and the two trailing marker
  bits were folded into the value), and every mux_rate this crate serialized emitted a forced bit
  in the marker position and failed re-parse (#1049).
- `ProgramStreamMap`: `elementary_stream_map_length` was read from (and written at) a fixed offset
  immediately after `program_stream_info_length`, before the program descriptor loop — Table 2-41
  places it AFTER that loop. Any conformant PSM with a non-empty program descriptor loop
  misparsed the first descriptor's bytes as the ES map length, and every PSM this crate serialized
  with program descriptors was non-conformant (#1050).
- `ProgramStreamMap::serialize_into`: `pseudo_descriptor_length` was written
  as `1 + descriptors.len() as u8` — the cast happened before the add, so a
  255-byte descriptor overflowed the addition (panicking in debug, wrapping
  in release) instead of being checked (#1129).
- `program_stream_map_length` (spec max 1018, Table 2-41),
  `program_stream_info_length`, `elementary_stream_map_length`, and
  `ES_info_length` were written with unchecked `as u16` casts (#1129).
- `SystemHeader`'s `stream_loop_len` accumulated in `u16`, overflowing
  (debug panic; an out-of-bounds buffer write in release, since
  `serialized_len()` used the same wrapped value the stream loop then wrote
  past) for more than 10 921 caller-constructed bounds. Now accumulated in
  `usize`, with `serialize_into` separately rejecting a total that does not
  fit the 16-bit `header_length` wire field (#1129).

## [0.4.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806). No public API or behaviour change.

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

## [0.1.3] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.2] — 2026-06-27

### Changed
- Depend on `mpeg-pes` (renamed from `dvb-pes`); no behaviour change.

## [0.1.1] — 2026-06-20

### Fixed

- `program_stream::parse_pack` — integer-underflow panic on adversarial input.
  When a `system_header` is present and `find_next_boundary` returns a
  `pack_start_code`/`program_end_code` offset *before* the system header's
  serialized length, `boundary - pes_start` subtracted with overflow. Now clamps
  via `saturating_sub`. Found by the new cargo-fuzz `mpeg_ps` target; regression
  test embeds the minimized artifact (#277).

## [0.1.0] — 2026-06-19

### Added

- `PackHeader` — parser+serializer for the MPEG-1/2 Program Stream pack header
  (ISO/IEC 13818-1 §2.5.3.3, Table 2-39): `pack_start_code` `0x000001BA`,
  42-bit SCR (33-bit base + 9-bit extension), 22-bit `program_mux_rate`,
  `pack_stuffing_length` + stuffing bytes, and reserved bits with marker
  validation.
- `SystemHeader` — parser+serializer for the optional system header
  (ISO/IEC 13818-1 §2.5.3.5, Table 2-40): `rate_bound`, `audio_bound`/`video_bound`,
  constraint flags, and per-stream P-STD buffer bounds with the
  `stream_id == 0xB7` extension form.
- `ProgramStreamMap` — parser+serializer for the Program Stream Map
  (ISO/IEC 13818-1 §2.5.4, Table 2-41): `map_stream_id` `0xBC`,
  elementary stream descriptor loop with `stream_id_extension` support,
  and CRC-32 (MPEG-2) validation via `dvb_common::crc32_mpeg2`.
- `program_stream::parse_pack` / `parse_all_packs` — pack walker that
  reassembles `PackHeader` → optional `SystemHeader` → PES packets
  (parsed via `dvb-pes`), respecting pack boundaries.
- `Scr` — 42-bit System Clock Reference newtype with `ticks()` and `seconds()`
  accessors (27 MHz units).
- Two runnable examples: `parse_pack_header` (inline bytes) and `walk_ps`
  (real `.mpg` fixture).
- Real-fixture test (`tests/fixture_ps.rs`) over a committed ffmpeg-muxed
  MPEG-2 Program Stream, validating byte-exact round-trip of every
  `PackHeader` and the `SystemHeader`.
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
