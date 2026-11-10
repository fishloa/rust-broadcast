---
status: DRAFT
---
# dvb-t2mi 13.1.0

_Released 2026-11-10._

Lockstep minor (13.0.0 -> 13.1.0). No breaking changes. The shared summary is in the lockstep note [v13.1.0](v13.1.0.md); this note lists what changed in `dvb-t2mi`.

## Changed
- The `crc` module doc now states that the T2-MI trailer check is CRC-32/MPEG-2 and calls `broadcast_common::crc32_mpeg2`; the code is unchanged.

## Dependencies
- `broadcast-common` stays at 11.0.x.

---

Published from tag `v13.1.0`.
