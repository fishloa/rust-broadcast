---
status: DRAFT
---
# dvb-tools 13.1.0

_Released 2026-11-10._

Lockstep minor (13.0.0 -> 13.1.0). No breaking changes. The shared summary is in the lockstep note [v13.1.0](v13.1.0.md); this note lists what changed in `dvb-tools`.

## Fixed
- File readers (`dump`, `pids`, ...) count the aligned packets at the end of the capture, and a capture shorter than five packets, which `TsResync` could not confirm before end of input.

## Dependencies
- `mpeg-ts` 0.7.0 -> 0.7.1.
- `dvb-si` 13.0.0 -> 13.1.0.

---

Published from tag `v13.1.0`.
