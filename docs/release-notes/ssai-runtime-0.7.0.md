# ssai-runtime 0.7.0

_Released 2026-11-15._

- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).

### Changed
- BREAKING: one splice-point conditioner. `condition_splice_point(requested_pts: Clock33, candidates: &[Clock33], max_delta: Duration)` replaces both the linear `condition_splice_point(u64, &[u64], u64)` and `condition_splice_point_wrapping(.., modulus)` (audit 19, issue #2726): the linear twin measured a cue just before the 33-bit wrap against a boundary just after it as ~8.6 × 10⁹ ticks and dropped the break, and a caller picking it (it came first in the docs) got a wrong nearest boundary. Every instant is now a `broadcast_common::clock33::Clock33` and the tolerance a `core::time::Duration` (rounded up to 90 kHz ticks), so there is no unit or wrap left for a caller to get wrong. `ConditionedSplicePoint::{requested_pts, snapped_pts}` are `Clock33`. `PTS_MODULUS_33` and `Error::PtsOutOfRange` are deleted (the shared `clock33::WRAP_33BIT` and `Clock33::try_new` own them).
- BREAKING: `SessionStore::expire_idle(ttl)` takes the idle lifetime as a `core::time::Duration` instead of `ttl_us: u64` (#2693).
### Changed
- BREAKING: `SessionStore::advance` takes a `broadcast_common::stage::Timestamp` (the caller's own monotonic clock, as every sans-IO engine takes) instead of `now_us: u64`; `expire_idle`'s TTL stays in microseconds.

---

Published from tag `ssai-runtime-v0.7.0`.
