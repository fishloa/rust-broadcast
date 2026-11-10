---
status: DRAFT
---
# dvb-bbframe 13.1.0

_Released 2026-11-10._

Lockstep minor (13.0.0 -> 13.1.0). No breaking changes. The shared summary is in the lockstep note [v13.1.0](v13.1.0.md); this note lists what changed in `dvb-bbframe`.

## Changed
- The `crc` module doc now states that it is CRC-8/DVB-S2 (0xD5, 8-bit), a different check from `broadcast_common`'s CRC-32/MPEG-2; the code is unchanged.

## Fixed
- `CarryOverExtractor`/`BbframePump` reset the NM CRC-8 chain and any pending partial UP when a frame is skipped (bad BBHEADER, wrong mode, invalid UPL, non-TS), so the next intact UP is no longer emitted with the transport error indicator set.
- A `SYNCD=0xFFFF` frame no longer drops the data-field bytes that follow the UP it completes; whole UPs and a trailing partial are kept, in both NM and HEM.

## Dependencies
- `broadcast-common` stays at 11.0.x.

---

Published from tag `v13.1.0`.
