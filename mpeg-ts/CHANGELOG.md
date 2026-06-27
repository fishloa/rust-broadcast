# Changelog

## [Unreleased]

## [0.1.0] — 2026-06-27

### Added
- Initial release: extracted from `dvb-si` at the 8.0.0 breaking boundary.
- `TsPacket` + `AdaptationField` + `PcrValue` — ITU-T H.222.0 §2.4.3.2 TS packet parse/serialize.
- `SectionReassembler` — per-PID PSI section assembly from TS payloads, with continuity-counter tracking and duplicate-version suppression.
- `SectionPacketizer` / `SiMux` — packetize PSI sections back into TS packets.
- `TsResync` — lost-sync recovery via sliding-window 0x47 search.
- `TsPacketBuf` — owned aligned 188-byte buffer type (zero-copy hand-off across async boundaries).
- `Pid` — typed 13-bit PID newtype with well-known constants (PAT, CAT, TSDT, NULL, NIT, SDT, EIT, TDT/TOT, …).
- `no_std` + `alloc`: suitable for embedded targets with a heap. Feature flags: `std` (default), `serde`.
