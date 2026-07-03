# Changelog

All notable changes to `mpeg-ps` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
