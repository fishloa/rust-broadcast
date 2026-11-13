# rist-runtime 0.6.0

_Released 2026-11-14._

Major-class release (0.5 -> 0.6) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.5.0 -> 0.6.0.

## Fixed
- `arq::Receiver::feed` resyncs, after the usual confirmed run, when a sender restarts at a lower sequence further behind than `max_gap` instead of dropping every packet as a late duplicate until the sequence passes the old highest again; a gap of exactly half the sequence space counts as a resync (backward) jump.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `rtp-packet` 0.6 -> 0.7.

---

Published from tag `rist-runtime-v0.6.0`.
