# timed-metadata 0.11.0

_Released 2026-11-15._

Minor-class release (0.10 -> 0.11). A `Timeline` has one constructor (a `with_`-prefixed one that was really a constructor is gone), the RFC 3339 renderer that clamped an out-of-range instant and printed a fabricated date is replaced by the fallible one, and timescales are the shared `Timescale`.

Version: 0.10.0 -> 0.11.0.

## Breaking changes and migration

### `Timeline::new` takes the anchor

`Timeline::with_anchor(anchor)` is `Timeline::new(anchor)` (a constructor is not named like a builder setter), the no-argument `Timeline::new()` is gone, and `set_anchor` is the one later change; start from `Timeline::default()` for a session that is anchored later.

```rust
// before
let timeline = Timeline::with_anchor(anchor);
let plain = Timeline::new();
// after
let timeline = Timeline::new(anchor);
let plain = Timeline::default();
```

### The infallible RFC 3339 twin is gone

`anchor::format_rfc3339_ms` and `TimeAnchor::rfc3339` are removed (`TimeAnchor::rfc3339` clamped an out-of-range instant and printed a fabricated date); `try_format_rfc3339_ms` and `TimeAnchor::try_rfc3339` are the one each, so an unrepresentable instant is `Error::TimestampOutOfRange`.

```rust
// before
let text = format_rfc3339_ms(ms);
let text = anchor.rfc3339(t);
// after
let text = try_format_rfc3339_ms(ms)?;
let text = anchor.try_rfc3339(t)?;
```

### Timescales are `Timescale`

`convert::EmsgConfig::timescale` and `convert::SegmentTiming::timescale` are `Timescale` (was `u32`), and `Error::EmsgTimescaleMismatch` carries `Timescale`s, following `mp4_emsg::EmsgBox::timescale`.

```rust
// before
let config = EmsgConfig { timescale: 90_000, .. };
// after
let config = EmsgConfig::new(Timescale::try_new(90_000)?, presentation, event_duration, value, id);
```

### `EmsgConfig` and `TimedEvent` are `#[non_exhaustive]`

Build them with `EmsgConfig::new(..)` and `TimedEvent::new(kind, source)` + `with_id`/`with_at`/`with_duration` instead of struct literals.

```rust
// before
let event = TimedEvent { kind, source, id: 1, .. };
// after
let event = TimedEvent::new(kind, source).with_id(1);
```

## Fixes

- `TimeAnchor::media_to_epoch_ms` floors the millisecond offset instead of truncating it toward zero, so an instant within a millisecond *before* the anchor no longer rounds up to the anchor's millisecond (a 1 ms skew that made the mapping asymmetric around the anchor).

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-hls` 0.7 -> 0.8, `cc-data` 0.9 -> 0.10, `dvb-vbi` 0.7 -> 0.8, `mp4-emsg` 0.7 -> 0.8, `scte35-splice` 6 -> 7.

---

Published from tag `timed-metadata-v0.11.0`.
