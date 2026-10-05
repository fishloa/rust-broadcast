# playout-runtime 0.2.0

_Released 2026-10-05._

Minor release of the sans-IO linear playout core: one bug fix, documentation of a unit precondition that was previously only implied, and the dependency epoch moves that force the version bump. **Upgrade if you build SCTE-35 `splice_insert` cues on a channel that runs continuously**, because a cue near the 33-bit PTS wrap used to fail. **Breaking in the dependency sense:** `scte35::build_splice_insert` returns a `scte35_splice::commands::SpliceInsert`, so this crate's public API names `scte35-splice` types and the move from 2.1 to 3.0 is a new epoch for callers (see [scte35-splice-3.0.0.md](scte35-splice-3.0.0.md)); it also builds on [ssai-runtime-0.2.0.md](ssai-runtime-0.2.0.md). There is no source-level API removal in this crate itself.

## Fix: cues across the PTS wrap (#1126)

`scte35::build_splice_insert` now conditions on the 33-bit SCTE-35 circle through `ssai_runtime::splice::condition_splice_point_wrapping`. `requested_pts` and the candidates are measured modulo 2^33, so a cue just before the wrap whose nearest real boundary is just after it snaps as a 150-tick `After` snap. Before, it failed with `NoAlignedBoundary` once per roughly 26.5 hours. The returned `ConditionedSplicePoint`'s delta and direction are circular, while its `requested_pts` and `snapped_pts` stay in the caller's own units (an unwrapped channel clock stays unwrapped). The `SpliceInsert`'s `splice_time` carries the snapped value reduced to 33 bits by `SpliceTime::with_pts`.

## Documentation made precise (no behaviour change)

- **Unit precondition (#1126).** `requested_pts`, every candidate, `max_delta_ticks` and `break_duration_ticks` given to `build_splice_insert` must already be 90 kHz tick counts (ANSI/SCTE 35 §9.8.1/§9.8.2). The function does no unit conversion or validation, so a channel clock in 27 MHz, milliseconds or nanoseconds produced a structurally valid cue that was silently wrong by a fixed factor. `ScheduleEntry::planned_start`/`source_start_pts` now document the same requirement; convert first with `scte35_splice::time::duration_to_ticks` if your clock is not 90 kHz. If you were passing a non-90 kHz clock, your existing cues are wrong and stay wrong until you convert.
- **`TransitionPlan::rebase`** requires `source_pts` to be already unwrapped. A source whose own PTS wraps mid-entry (a 33-bit MPEG PTS) must be unrolled first by the caller, because `rebase` is pure and cannot detect a wrap.

## Dependency changes

From `git diff playout-runtime-v0.1.0..HEAD -- playout-runtime/Cargo.toml`: `broadcast-common` 9.3 to 9.4, `ssai-runtime` 0.1 to 0.2, `scte35-splice` 2.1 to 3.0. MSRV 1.95.0.

---

Published from tag `playout-runtime-v0.2.0`.
