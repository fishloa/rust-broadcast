# Changelog

## [Unreleased]

## [0.1.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.0] — 2026-06-27

### Added
- Initial release: extracted from `dvb-si` at the 8.0.0 breaking boundary.
- `TsPacket` + `AdaptationField` + `PcrValue` — ITU-T H.222.0 §2.4.3.2 TS packet parse/serialize.
- `SectionReassembler` — per-PID PSI section assembly from TS payloads, with continuity-counter tracking and duplicate-version suppression.
- `SectionPacketizer` / `SiMux` — packetize PSI sections back into TS packets.
- `TsResync` — lost-sync recovery via sliding-window 0x47 search.
- `OwnedTsPacket` — owned aligned 188-byte buffer type (zero-copy hand-off across async boundaries), with `scrambling_control()`/`adaptation_field_control()` typed accessors and a `discontinuity` field.
- `ScramblingControl` — typed 2-bit `transport_scrambling_control` enum (`NotScrambled`/`Reserved`/`EvenKey`/`OddKey`); cited to ETSI TS 100 289 §5.1 + H.222.0 Table 2-4. `name()` + `Display` (#204).
- `AdaptationFieldControl` — typed `adaptation_field_control` enum (`Reserved`/`PayloadOnly`/`AdaptationOnly`/`AdaptationAndPayload`); H.222.0 Table 2-5. `name()` + `Display` (#204).
- `TsHeader::{scrambling_control, adaptation_field_control}` — typed accessors on the zero-copy borrowed packet header.
- `iter_packets(&[u8])` — free helper that walks a buffer of concatenated 188-byte packets, yielding `TsPacket` items.
- `extract_ts_payload(&[u8])` — free helper returning the payload slice past header+adaptation from a raw packet.
- `Pid` — typed 13-bit PID newtype with well-known constants (PAT, CAT, TSDT, NULL, NIT, SDT, EIT, TDT/TOT, …).
- `no_std` + `alloc`: suitable for embedded targets with a heap. Feature flags: `std` (default), `serde`.
