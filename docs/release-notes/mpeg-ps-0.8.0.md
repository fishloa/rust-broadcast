# mpeg-ps 0.8.0

_Released 2026-11-14._

Major-class release (0.7 -> 0.8) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.7.0 -> 0.8.0.

## Fixed
- `ProgramStreamMap` parse and serialize now count `CRC_32` in `program_stream_map_length` as ISO/IEC 13818-1 Table 2-41 requires; before, any real PSM failed to parse (`MapLengthOverflow`, or a `CrcMismatch` that lost the whole pack) and serialize wrote a length 4 short.
- The `PackHeader` and `SystemHeader` output-buffer pre-checks used the parse-side `need_in`; a too-small output buffer now reports `Error::OutputBufferTooSmall` as intended instead of `Error::BufferTooShort`.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `mpeg-pes` 0.7 -> 0.8.

---

Published from tag `mpeg-ps-v0.8.0`.
