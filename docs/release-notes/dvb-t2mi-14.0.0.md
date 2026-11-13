# dvb-t2mi 14.0.0

_Released 2026-11-14._

Major-class release (13 -> 14) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 13.0.0 -> 14.0.0.

## Changed
- The `crc` module doc now states that the T2-MI trailer check is CRC-32/MPEG-2 and calls `broadcast_common::crc32_mpeg2`; the code is unchanged (c6 C6-12).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-bbframe` 13 -> 14, `mpeg-ts` 0.7 -> 0.8.

---

Published from tag `v14.0.0`.
