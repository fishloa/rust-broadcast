# cc-data 0.9.0

_Released 2026-11-14._

Major-class release (0.8 -> 0.9) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.8.0 -> 0.9.0.

## Fixed
- 608 Text Restart (and RTD) no longer erase the caption screen, and characters typed in text mode no longer appear in the channel's caption text: text mode now has its own screen.
- A 608 control pair after a null pair is no longer suppressed as a doubled control code.
- A field-1 608 pair whose first byte is in `0x01`-`0x0F` (the XDS range) no longer prints its second byte as text.
- 708 `SetPenLocation` back over existing text now overwrites the cell instead of appending at the end of the row.
- `CcData::cea608()`/`cea708()` yield nothing when `process_cc_data_flag` is clear (ETSI TS 101 154 Table B.9: discard); new `CcData::processed_triplets()`.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `cc-data-v0.9.0`.
