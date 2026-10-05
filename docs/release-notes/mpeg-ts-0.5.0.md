# mpeg-ts 0.5.0

_Released 2026-10-05._

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

---

Published from tag `mpeg-ts-v0.5.0`.
