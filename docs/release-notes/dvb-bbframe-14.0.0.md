# dvb-bbframe 14.0.0

_Released 2026-11-14._

Major-class release (13 -> 14) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 13.0.0 -> 14.0.0.

## Changed
- The `crc` module doc now states that it is CRC-8/DVB-S2 (0xD5, 8-bit), a different check from `broadcast_common`'s CRC-32/MPEG-2; the code is unchanged (c6 C6-12).

## Fixed
- `CarryOverExtractor`/`BbframePump` reset the NM CRC-8 chain and any pending partial UP when a frame is skipped (bad BBHEADER, wrong mode, invalid UPL, non-TS), so the next intact UP is no longer emitted with the transport error indicator set (#1944).
- A `SYNCD=0xFFFF` frame no longer drops the data-field bytes that follow the UP it completes; whole UPs and a trailing partial are kept, in both NM and HEM (#1949).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `v14.0.0`.
