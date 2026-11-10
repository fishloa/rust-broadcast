---
status: DRAFT
---
# rist-runtime 0.5.1

_Released 2026-11-10._

Patch release: ARQ receiver resync fix.

Version: 0.5.0 -> 0.5.1.

## Fixed
- `arq::Receiver::feed` resyncs, after the usual confirmed run, when a sender restarts at a lower sequence further behind than `max_gap` instead of dropping every packet as a late duplicate until the sequence passes the old highest again; a gap of exactly half the sequence space counts as a resync (backward) jump.

---

Published from tag `rist-runtime-v0.5.1`.
