# playout-runtime 0.2.0

_Released 2026-10-05._

### Fixed
- `scte35::build_splice_insert` conditions on the 33-bit SCTE-35 circle
  (`ssai_runtime::splice::condition_splice_point_wrapping`): `requested_pts`
  and the candidates are measured modulo 2^33, so a cue just before the PTS wrap
  whose nearest real boundary is just after it snaps (a 150-tick `After` snap)
  instead of failing with `NoAlignedBoundary` once per ~26.5 h; the returned
  `ConditionedSplicePoint`'s delta/direction are circular, and its
  `requested_pts`/`snapped_pts` stay in the caller's units (#1126).

### Documentation
- `ScheduleEntry::planned_start`/`source_start_pts` and
  `scte35::build_splice_insert` now document, as a mandatory precondition,
  that a value reaching `build_splice_insert` must already be a 90 kHz tick
  count (ANSI/SCTE 35 §9.8.1/§9.8.2) — previously only described as "the
  natural choice", not required, so a caller using a different channel clock
  got a structurally valid but silently wrong-by-a-fixed-factor cue with no
  error (#1126).
- `TransitionPlan::rebase` now documents that `source_pts` must already be
  unwrapped: a source whose own PTS numbering wraps mid-entry (e.g. a 33-bit
  MPEG PTS) needs unrolling (`transmux::Timeline` already does this) before
  being passed here, since `rebase` is pure and has no state to detect a
  wrap itself (#1126). No behaviour change — both are documentation fixes.

---

Published from tag `playout-runtime-v0.2.0`.
