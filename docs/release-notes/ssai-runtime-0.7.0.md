# ssai-runtime 0.7.0

_Released 2026-11-15._

Minor-class release (0.6 -> 0.7). One splice-point conditioner replaces the linear one and its wrapping twin, so a cue just before the 33-bit wrap can no longer be dropped against a boundary just after it, and the store's clock and idle lifetime are the workspace's typed values.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

### `condition_splice_point` is the one conditioner

`condition_splice_point(requested_pts: Clock33, candidates: &[Clock33], max_delta: Duration)` replaces both the linear `condition_splice_point(u64, &[u64], u64)` and `condition_splice_point_wrapping(.., modulus)`: the linear twin measured a cue just before the 33-bit wrap against a boundary just after it as ~8.6 × 10⁹ ticks and dropped the break, and a caller picking it (it came first in the docs) got a wrong nearest boundary. Every instant is now a `Clock33` and the tolerance a `Duration` (rounded up to 90 kHz ticks). `ConditionedSplicePoint::{requested_pts, snapped_pts}` are `Clock33`, and `PTS_MODULUS_33`/`Error::PtsOutOfRange` are deleted (the shared `clock33::WRAP_33BIT` and `Clock33::try_new` own them).

```rust
// before
let point = condition_splice_point(requested_pts, &candidates, 45_000)?;
// after
let point = condition_splice_point(Clock33::try_new(requested_pts)?, &candidates, Duration::from_millis(500))?;
let snapped: u64 = point.snapped_pts.ticks();
```

### The store takes a `Timestamp` and a `Duration`

`SessionStore::advance` takes a `broadcast_common::stage::Timestamp` (the caller's own monotonic clock, as every sans-IO engine takes) instead of `now_us: u64`, and `SessionStore::expire_idle` takes the idle lifetime as a `Duration` instead of `ttl_us: u64`.

```rust
// before
store.advance(now_us);
store.expire_idle(30_000_000);
// after
store.advance(Timestamp::from_micros(now_us));
store.expire_idle(Duration::from_secs(30));
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-hls` 0.7 -> 0.8, `mp4-emsg` 0.7 -> 0.8, `scte35-splice` 6 -> 7.

---

Published from tag `ssai-runtime-v0.7.0`.
