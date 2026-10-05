# Changelog

## [Unreleased]

## [0.5.0] - 2026-10-05
### Changed (breaking)
- `OwnedTsPacket`'s fields (`raw`, `pid`, `pusi`, `has_adaptation`, `has_payload`, `tei`, `scrambling`, `continuity_counter`) are now **private**, with getters of the same names (`raw()` returning `&[u8; 188]`, `into_raw()`, `pid()`, `pusi()`, `has_adaptation()`, `has_payload()`, `tei()`, `scrambling()`, `continuity_counter()`). A public copy of a header field a caller could assign drifted silently from `raw`, which is what serialises (audit r01-W8, #1074). `discontinuity` stays public: it is caller metadata, not a copy of wire bytes. `ts-fix` and `transmux` call sites updated.
- Requires `broadcast-common` 9.4 (`broadcast_common::len`).
- `OwnedTsPacket::serialize_with_payload` now returns
  `Result<[u8; TS_PACKET_SIZE], Error>` instead of `[u8; TS_PACKET_SIZE]`,
  rejecting a payload over 184 bytes (the new
  `OwnedTsPacket::MAX_PAYLOAD_LEN`) instead of silently truncating it (#1129).
  A new `Error::PayloadTooLarge` variant is added.

### Fixed
- `OwnedTsPacket::serialize_with_payload` silently truncated a payload over
  184 bytes (`payload.len().min(184)`) with no signal; it now errors (#1129).
- `AdaptationField::serialize_into` wrote `transport_private_data_length`
  with an unchecked `as u8` cast, wrapping to a short length for data over
  255 bytes while the full data was still copied after it (#1129).
- `mux::SiMux` keyed its scheduler entries by PID alone, so two distinct
  tables sharing a PID by spec (TDT/TOT on `0x0014`, SDT/BAT on `0x0011`,
  EIT present/following vs. schedule sub-tables on `0x0012`) collided:
  `upsert_tot` after `upsert_tdt` silently replaced the TDT entry and it
  never appeared in the output. Entries are now keyed by `(pid, table_id)`,
  and every entry on one PID shares a single `SectionPacketiser` (and so one
  continuity counter), so two tables on a shared PID stay CC-continuous
  (#1000). Verified against TSDuck's own `tsp -P tables`/`tsanalyze`
  (`mpeg-ts/tests/tsduck_simux_oracle.rs`).
- `SectionPacketiser::packetise_into` could set PUSI=1 with `pointer_field ==
  183` when a section's tail exactly filled a packet's PUSI payload capacity
  — a pointer past the packet's own payload, wrongly claiming a section
  starts within it (H.222.0 §2.4.4). A related bound was also missing: a
  non-PUSI continuation packet at the same boundary could copy one byte past
  the next section's start, which the reassembler then silently dropped,
  corrupting that next section (#1074).
- `Section::serialize_into` indexed its output buffer by `payload.len()`
  while sizing it from the independently-settable `section_length` field;
  a hand-edited `Section` whose two disagreed could panic with an
  out-of-bounds slice index (oversized payload) or silently emit a
  short/misframed section with a wrong CRC (undersized payload). Now
  validated up front and rejected with a new `Error::SectionPayloadLengthMismatch`
  (#1074).
- `TsPacket::parse` silently `.min()`-truncated an adaptation field whose
  declared `adaptation_field_length` didn't fit the packet, while
  `OwnedTsPacket::adaptation_field` already rejected the same bytes as
  `None` — two views of one wire format disagreeing on a malformed AF.
  `TsPacket::parse` now also rejects it (#1074).
- `OwnedTsPacket::set_pcr` only checked `adaptation_field_length >= 1`
  (room for the flags byte), not `>= 1 + 6` (room for the PCR itself); a
  malformed adaptation field with the PCR flag set but too short a declared
  length let it write 6 PCR bytes at a fixed offset, spilling past the
  declared adaptation field into the payload. Now requires
  `adaptation_field_length >= 7` (#1074).

### Changed
- `SectionPacketiser::packetise_into` finds each packet's next section start with a forward-only cursor instead of rescanning the start list from the front (audit r01-O1, #1074); output is unchanged (differential round-trip test over 400 boundary-straddling sections). r01-O2 (per-poll section split) and r01-O4 (`TsResync::feed_into`) are deliberately not done: no measured hot path, and O4 would add public API speculatively.
- Named the remaining flag-bit literals in `section.rs`/`ts.rs` (`CURRENT_NEXT_MASK`, `PCR_RESERVED_BITS`) and replaced hand-masked low-byte splits with `to_be_bytes` (audit r01-W15, #1074). The PES header flag bits in `mpeg-pes` are named the same way.
- `PusiReassembler` is now a PID filter over `broadcast_common::pusi::PusiAccumulator`, the same accumulator `mpeg-pes::PesAssembler` uses, replacing two diverged copies of the PUSI-gating rule. One observable difference: a PUSI that closes an *empty* in-progress unit no longer returns `Some(vec![])` (audit r01-W13, #1074).
- `mux::split_sections`, `SectionReassembler`'s two internal section-length
  reads, and `Section::parse` shared one duplicated 12-bit `section_length`
  decode; extracted to a single `ts::section_total_len` helper (#1074).
- `extract_ts_payload` re-implemented the adaptation-field-skip walk that
  `TsPacket::parse` already does, with its own `adaptation_field_control`
  bit mask; both now share one `adaptation_field_skip` helper, and
  `extract_ts_payload` reuses `TsHeader::parse` instead of re-deriving the
  flag bits from the raw header byte (#1074).
- Named several bit-field masks in `section.rs`'s and `ts.rs`'s
  adaptation-field-extension code that were previously inline hex literals
  (#1074).

## [0.4.1] - 2026-09-26

### Security
Fixes GHSA-74gr-mvr4-2qp8.

### Fixed
- `PusiReassembler` now ignores non-PUSI payloads until the first PUSI is received (ISO/IEC 13818-1 §2.4.3.2), preventing unrelated bytes from being prepended to reassembled units.
- `PusiReassembler` enforces a 65536-byte cap on accumulated unit size; units exceeding this are discarded and reassembly restarts at the next PUSI boundary.

## [0.4.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.3.1] - 2026-07-27
### Changed
- Internal, test-only: dropped a redundant `&` in a `fixture_roundtrip`
  `eprintln!` argument to satisfy `clippy::needless_borrows_for_generic_args`
  on the pinned canary toolchain. No public API or behaviour change.

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
