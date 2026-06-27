# Changelog

## Unreleased

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
