# Changelog

## [Unreleased]

## [0.3.0] - 2026-07-21

### Changed (BREAKING)
- Renamed `mux::SectionPacketiser` and its `packetise`/`packetise_into`
  methods to British spelling (issue #663; both were previously spelled with
  a "z"). Pure rename — behaviour-preserving, no functional change.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.2] — 2026-07-01
### Added
- `PusiReassembler` — generic PUSI-delimited payload reassembler for non-PSI PID
  data (e.g. a DASH `emsg` box on reserved PID `0x0004`, ISO/IEC 23009-1:2022
  §5.10.3.3.5): accumulates payload bytes across a PUSI-delimited run and yields
  the complete unit (stuffing lives in the adaptation field, so accumulated
  payload = clean box bytes).
- `examples/edit_packet.rs` — walk-through demonstrating the write/edit API:
  read a PCR-bearing packet, mutate PCR and CC, build a null packet, and
  round-trip a packet.
- `AdaptationField::stuffing_len: usize` — number of trailing `0xFF` stuffing
  bytes padding the adaptation-field body out to its `adaptation_field_length`
  (ISO/IEC 13818-1 §2.4.3.4). Captured on parse and re-emitted on serialize so a
  stuffed adaptation field round-trips **byte-identical**. Additive on the
  `#[non_exhaustive]` struct; construct with `stuffing_len: 0` for no stuffing.
- `Pcr::from_27mhz(ticks: u64) -> Pcr` — construct a PCR from an absolute 27 MHz
  clock value (ISO/IEC 13818-1 §2.4.3.5).
- `Pcr::to_field_bytes(self) -> [u8; 6]` — serialize PCR to the 6-byte wire
  field; exact inverse of `Pcr::parse`.
- `ScramblingControl::to_bits(self) -> u8` — 2-bit scrambling-control code
  (ETSI TS 100 289 §5.1, H.222.0 Table 2-4).
- `AdaptationFieldControl::to_bits(self) -> u8` — 2-bit adaptation_field_control
  code (H.222.0 Table 2-5).
- `AdaptationFieldControl::to_flags(self) -> (bool, bool)` — `(has_adaptation,
  has_payload)` pair for constructing TS packet headers.
- `AdaptationField::serialize_into` — full symmetric serializer for the
  adaptation field, including all optional sub-structures.
- `Ltw`, `SeamlessSplice`, `AdaptationFieldExtension` — typed sub-structures for
  the adaptation field extension (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5): LTW
  (2-byte: valid_flag + 15-bit offset), piecewise_rate (22-bit), and seamless
  splice (33-bit DTS-format next AU DTS with 4-bit splice_type).
- `AdaptationField::transport_private_data: Option<&[u8]>` — opaque private data
  blob (correct API per spec).
- `AdaptationField::extension: Option<AdaptationFieldExtension>` — typed extension
  sub-structure.
- `OwnedTsPacket::null_packet(cc: u8) -> [u8; 188]` — construct a null packet
  (PID 0x1FFF, no payload) per ISO/IEC 13818-1 §2.4.1.
- `OwnedTsPacket::set_continuity_counter(packet, cc)` — overwrite the CC in an
  existing 188-byte packet buffer (bits [3:0] of byte 3).
- `OwnedTsPacket::set_pcr(packet, pcr) -> Result<()>` — overwrite the PCR field
  in an existing adaptation field.
- `OwnedTsPacket::adaptation_field` — decode the adaptation field from the owned
  buffer.
- All new types are symmetric: `parse` + `serialize_into` with round-trip tests.

### Changed
- `AdaptationField` now carries a `'a` lifetime (borrows `transport_private_data`
  from the packet buffer); it is no longer `Copy` (use `Clone`).

### Fixed
- `AdaptationField::serialize_into` now reproduces the trailing `0xFF` stuffing
  instead of dropping it, so parse → serialize is byte-identical for real
  broadcast adaptation fields (PCR + stuffing, pure stuffing, etc.). Verified on
  the committed `m6-single.ts` capture and a France-TNT-derived stuffed-AF
  fixture (every unscrambled adaptation field round-trips byte-for-byte).

### Added
- `Pcr::from_27mhz(ticks: u64) -> Pcr` — construct a PCR from an absolute 27 MHz
  clock value (ISO/IEC 13818-1 §2.4.3.5).
- `Pcr::to_field_bytes(self) -> [u8; 6]` — serialize PCR to the 6-byte wire
  field; exact inverse of `Pcr::parse`.
- `ScramblingControl::to_bits(self) -> u8` — 2-bit scrambling-control code
  (ETSI TS 100 289 §5.1, H.222.0 Table 2-4).
- `AdaptationFieldControl::to_bits(self) -> u8` — 2-bit adaptation_field_control
  code (H.222.0 Table 2-5).
- `AdaptationFieldControl::to_flags(self) -> (bool, bool)` — `(has_adaptation,
  has_payload)` pair for constructing TS packet headers.
- `AdaptationField::serialize_into` — full symmetric serializer for the
  adaptation field, including all optional sub-structures.
- `Ltw`, `SeamlessSplice`, `AdaptationFieldExtension` — typed sub-structures for
  the adaptation field extension (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5): LTW
  (2-byte: valid_flag + 15-bit offset), piecewise_rate (22-bit), and seamless
  splice (33-bit DTS-format next AU DTS with 4-bit splice_type).
- `AdaptationField::transport_private_data: Option<&[u8]>` — opaque private data
  blob (correct API per spec).
- `AdaptationField::extension: Option<AdaptationFieldExtension>` — typed extension
  sub-structure.
- `OwnedTsPacket::null_packet(cc: u8) -> [u8; 188]` — construct a null packet
  (PID 0x1FFF, no payload) per ISO/IEC 13818-1 §2.4.1.
- `OwnedTsPacket::set_continuity_counter(packet, cc)` — overwrite the CC in an
  existing 188-byte packet buffer (bits [3:0] of byte 3).
- `OwnedTsPacket::set_pcr(packet, pcr) -> Result<()>` — overwrite the PCR field
  in an existing adaptation field.
- `OwnedTsPacket::adaptation_field` — decode the adaptation field from the owned
  buffer.
- All new types are symmetric: `parse` + `serialize_into` with round-trip tests.

### Changed
- `AdaptationField` now carries a `'a` lifetime (borrows `transport_private_data`
  from the packet buffer); it is no longer `Copy` (use `Clone`).

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
