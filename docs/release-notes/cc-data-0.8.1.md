---
status: DRAFT
---
# cc-data 0.8.1

_Released 2026-11-10._

Patch release: caption extraction fixes.

Version: 0.8.0 -> 0.8.1.

## Fixed
- 608 Text Restart (and RTD) no longer erase the caption screen, and characters typed in text mode no longer appear in the channel's caption text: text mode now has its own screen.
- A 608 control pair after a null pair is no longer suppressed as a doubled control code.
- A field-1 608 pair whose first byte is in `0x01`-`0x0F` (the XDS range) no longer prints its second byte as text.
- 708 `SetPenLocation` back over existing text now overwrites the cell instead of appending at the end of the row.
- `CcData::cea608()`/`cea708()` yield nothing when `process_cc_data_flag` is clear (ETSI TS 101 154 Table B.9: discard); new `CcData::processed_triplets()`.

---

Published from tag `cc-data-v0.8.1`.
