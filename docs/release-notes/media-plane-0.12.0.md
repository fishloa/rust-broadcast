# media-plane 0.12.0

_Released 2026-11-15._

Minor-class release (0.11 -> 0.12). Durations are `Duration` instead of bare seconds or floats, the configuration failures that used to assert are typed `Result` values (`ByteMerge::new`, `ByteTap::new`), the reconnect policy is the workspace's shared `ReconnectPolicy`, and a subtitle follower keeps the playlist number the leader handed it.

Version: 0.11.0 -> 0.12.0.

## Breaking changes and migration

### Durations are `Duration`

`SegmenterConfig::target_duration_secs: f64` is `target_duration: Duration` (set with `with_target_duration(Duration)`), `file_source::PassRange::span_secs`/`frame_secs` are `span`/`frame: Duration`, `segmenter::LeaderDurations` maps a leader segment number to a `Duration`, `segmenter::subtitle::LeaderPart::duration_secs` is `duration: Duration`, and `segmenter::audio::AudioFollower::try_new` takes its target duration as a `Duration`. A `Duration` cannot be negative, NaN or infinite, so the former non-finite/negative check is now a zero check.

```rust
// before
let config = SegmenterConfig::default().with_target_duration_secs(1.0);
let span = range.span_secs;
// after
let config = SegmenterConfig::default().with_target_duration(Duration::from_secs(1));
let span = range.span;   // Duration
```

### Part targets and reconnect offsets are `Duration`

`ProgramSegmenter::continue_after_reconnect` takes the media start as a `Duration` (it was `media_start_ns: u64`), `SegmenterConfig::part_target_ms: u32` is `part_target: Duration` (with `with_part_target(Duration)`), `SiblingTrunks::build_audio_followers` takes that `Duration`, and `SegmenterEvent::PartTargetMs(u32)` is `SegmenterEvent::PartTarget(Duration)`.

```rust
// before
segmenter.continue_after_reconnect(ns);
let config = SegmenterConfig::default().with_part_target_ms(250);
match event { SegmenterEvent::PartTargetMs(ms) => { .. } _ => {} }
// after
segmenter.continue_after_reconnect(Duration::from_nanos(ns));
let config = SegmenterConfig::default().with_part_target(Duration::from_millis(250));
match event { SegmenterEvent::PartTarget(d) => { .. } _ => {} }
```

### `ByteMerge::new` and `ByteTap::new` report their configuration errors

`ByteMerge::new` returns `Result<Self, MergeConfigError>` instead of panicking; its three assertions are the typed variants `ZeroSources`, `ZeroMaxQueued` and `FailoverOutOfRange`. `ByteTap::new` returns `Result<Self, TapError>` with `TapError::ZeroCapacity` (exposed as `media_plane::TapError`), so the same call works from a configuration validator. `SegmenterEvent::OutputFailed` carries a typed `TsHlsRejection` (`Hevc`, `Av1`, with `name()`/`reason()`) instead of a formatted message, so a host branches on the rejected codec rather than its text.

```rust
// before
let merge = ByteMerge::new(policy, 2, 8);
let tap = ByteTap::new(point, capacity);
match event { SegmenterEvent::OutputFailed(msg) => log(msg), _ => {} }
// after
let merge = ByteMerge::new(policy, 2, 8).expect("counts are valid");
let tap = ByteTap::new(point, capacity)?;
match event { SegmenterEvent::OutputFailed(rejection) => log(rejection.reason()), _ => {} }
```

### `ReconnectPolicy` is `broadcast_common::backoff::ReconnectPolicy`

`ingress::ReconnectPolicy` is the shared type (`backoff: BackoffSteps` + `max_attempts: Option<NonZeroU32>`); its old `new(max_attempts: NonZeroU32)` is `ReconnectPolicy::default().with_max_attempts(nonzero)`, and a `DialSupervisor` reads the bound through `ReconnectPolicy::exhausted`.

```rust
// before
let policy = ReconnectPolicy::new(NonZeroU32::new(10).unwrap());
// after
let policy = ReconnectPolicy::default().with_max_attempts(NonZeroU32::new(10).unwrap());
```

### The subtitle follower is numbered with the playlist's types

Its internal segment number is a `broadcast_hls::MediaSequence` (and its part index a `PartIndex`), matching the cuts a leader hands it; `LeaderDurations` keeps its `u32` key, since it holds a segment duration per leader segment for every follower, not a playlist number.

## Fixes

- `epoch_ms_to_media` floors its media tick like `TimeAnchor::media_to_epoch_ms` does instead of truncating toward zero, so a UTC instant just before a millisecond the forward direction floors onto no longer resolves to an inconsistent tick.
- Cold-ledger expiry adds the retention window with `checked_add`: an entry whose expiry does not fit the host clock stays in the ledger (it is never expired) rather than expiring at a saturated instant.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-hls` 0.7 -> 0.8, `caption-convert` 0.6 -> 0.7, `container-probe` 0.4 -> 0.5.
- `dvb-si` 14 -> 15, `scte35-splice` 6 -> 7, `timed-metadata` 0.10 -> 0.11, `transmux` 0.31 -> 0.32.

---

Published from tag `media-plane-v0.12.0`.
