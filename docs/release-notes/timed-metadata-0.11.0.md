# timed-metadata 0.11.0

_Released 2026-11-15._

### Dependencies
- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).

### Changed
- BREAKING: `Timeline::with_anchor(anchor)` is `Timeline::new(anchor)`: a constructor is not named like a builder setter, and the no-argument `Timeline::new()` is gone, so `new` is the one constructor and `set_anchor` the one later change. Migration: `Timeline::with_anchor(a)` becomes `Timeline::new(a)`, and `Timeline::new()` becomes `Timeline::default()` (#2665).
- BREAKING: `convert::EmsgConfig::timescale` and `convert::SegmentTiming::timescale` are `Timescale` (was `u32`), and `Error::EmsgTimescaleMismatch` carries `Timescale`s, following `mp4_emsg::EmsgBox::timescale` (#2657).
- BREAKING: the infallible RFC 3339 twin is gone: `anchor::format_rfc3339_ms` and `TimeAnchor::rfc3339` (which clamped an out-of-range instant and printed a fabricated date) are removed; `try_format_rfc3339_ms` and `TimeAnchor::try_rfc3339` are the one each, so an unrepresentable instant is `Error::TimestampOutOfRange`. Migration: `format_rfc3339_ms(ms)` becomes `try_format_rfc3339_ms(ms)` with `?`, `anchor.rfc3339(t)` becomes `anchor.try_rfc3339(t)` (issue #2724).

### Fixed
- `TimeAnchor::media_to_epoch_ms` floors the millisecond offset instead of truncating it toward zero, so an instant within a millisecond BEFORE the anchor no longer rounds up to the anchor's millisecond (a 1 ms skew that made the mapping asymmetric around the anchor) (issue #2724).

### Changed
- BREAKING: `convert::EmsgConfig` and `TimedEvent` are now `#[non_exhaustive]`; build them with `EmsgConfig::new(..)` and `TimedEvent::new(kind, source)` + `with_id`/`with_at`/`with_duration` instead of struct literals (#2664).

---

Published from tag `timed-metadata-v0.11.0`.
