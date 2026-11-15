# playout-runtime 0.7.0

_Released 2026-11-15._

- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).


### Changed
- BREAKING: `build_splice_insert` takes `requested_pts: Clock33`, `candidates: &[Clock33]`, `max_delta: Duration` and `break_duration: Option<Duration>` instead of raw `u64` tick counts (audit 19, issue #2726). It now delegates to the one `ssai_runtime::splice::condition_splice_point`; the returned `ConditionedSplicePoint` reports the reduced 33-bit instant and boundary that the emitted `splice_time().pts_time` carries, so the two can no longer disagree. `Error::BreakDurationTooLong`'s field is `nanos: u64` (the rejected duration in nanoseconds).
- BREAKING: `next_transition(schedule, now: Playhead)` takes the playhead as ticks on the schedule's own timescale (`Playhead::new(ticks, Timescale)`) instead of a bare `now_pts: u64`, so a media position cannot be read in one clock while it was produced on another (#2693).

---

Published from tag `playout-runtime-v0.7.0`.
