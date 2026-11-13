# mpeg-pes 0.8.0

_Released 2026-11-14._

Major-class release (0.7 -> 0.8) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.7.0 -> 0.8.0.

## Added
- `Error::BadMarker(&'static str)` for a wrong ESCR marker bit, `P-STD_buffer` `'01'` prefix or `program_packet_sequence_counter` marker bit.

## Fixed
- `PesPacket::parse` validates the ESCR, `P-STD_buffer` and `program_packet_sequence_counter` marker/prefix bits instead of accepting them and silently rewriting them on serialize.

## Also in this release

### Added
- `PusiAccumulator::len`/`is_empty`/`is_started` accessors; `PesAssembler` mirrors them.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `mpeg-pes-v0.8.0`.
