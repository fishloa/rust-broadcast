---
status: DRAFT
---
# mpeg-pes 0.7.1

_Released 2026-11-10._

Patch release: validate ESCR, P-STD_buffer and program_packet_sequence_counter marker/prefix bits.

Version: 0.7.0 -> 0.7.1.

## Added
- `Error::BadMarker(&'static str)` for a wrong ESCR marker bit, `P-STD_buffer` `'01'` prefix or `program_packet_sequence_counter` marker bit.

## Fixed
- `PesPacket::parse` validates the ESCR, `P-STD_buffer` and `program_packet_sequence_counter` marker/prefix bits instead of accepting them and silently rewriting them on serialize.

---

Published from tag `mpeg-pes-v0.7.1`.
