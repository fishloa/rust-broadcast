# playout-runtime 0.7.0

_Released 2026-11-15._

Minor-class release (0.6 -> 0.7). The splice-point conditioner is the shared one from `ssai-runtime`, so a cue just before the 33-bit wrap can no longer be measured against a boundary just after it, and every media position is a validated `Clock33` or a `Playhead` on the schedule's own timescale.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

### `build_splice_insert` takes clock values

`build_splice_insert` takes `requested_pts: Clock33`, `candidates: &[Clock33]`, `max_delta: Duration` and `break_duration: Option<Duration>` instead of raw `u64` tick counts. It delegates to `ssai_runtime::splice::condition_splice_point`, and the returned `ConditionedSplicePoint` reports the reduced 33-bit instant and boundary that the emitted `splice_time().pts_time` carries, so the two can no longer disagree. `Error::BreakDurationTooLong`'s field is `nanos: u64`.

```rust
// before
let (point, insert) = build_splice_insert(edge, id, requested_pts, &candidates, 45_000, Some(duration_ticks))?;
// after
let (point, insert) = build_splice_insert(
    edge, id,
    Clock33::try_new(requested_pts)?,
    &candidates.iter().map(|t| Clock33::try_new(*t)).collect::<Result<Vec<_>, _>>()?,
    Duration::from_millis(500),
    Some(Duration::from_secs(30)),
)?;
```

### `next_transition` takes a `Playhead`

`next_transition(schedule, now: Playhead)` takes the playhead as ticks on the schedule's own timescale (`Playhead::new(ticks, Timescale)`) instead of a bare `now_pts: u64`, so a media position cannot be read in one clock while it was produced on another.

```rust
// before
let next = next_transition(&schedule, now_pts);
// after
let next = next_transition(&schedule, Playhead::new(now_pts, Timescale::try_new(90_000)?));
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `mp4-emsg` 0.7 -> 0.8, `scte35-splice` 6 -> 7 (its `SpliceTime::pts_time` is the same validated `Clock33`), `ssai-runtime` 0.6 -> 0.7.

---

Published from tag `playout-runtime-v0.7.0`.
