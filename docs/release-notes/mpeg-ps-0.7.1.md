---
status: DRAFT
---
# mpeg-ps 0.7.1

_Released 2026-11-10._

Patch release: ProgramStreamMap length and output-buffer pre-check fixes.

Version: 0.7.0 -> 0.7.1.

## Fixed
- `ProgramStreamMap` parse and serialize now count `CRC_32` in `program_stream_map_length` as ISO/IEC 13818-1 Table 2-41 requires; before, any real PSM failed to parse (`MapLengthOverflow`, or a `CrcMismatch` that lost the whole pack) and serialize wrote a length 4 short.
- The `PackHeader` and `SystemHeader` output-buffer pre-checks used the parse-side `need_in`; a too-small output buffer now reports `Error::OutputBufferTooSmall` as intended instead of `Error::BufferTooShort`.

## Dependencies
- `mpeg-pes` 0.7.0 -> 0.7.1.

---

Published from tag `mpeg-ps-v0.7.1`.
