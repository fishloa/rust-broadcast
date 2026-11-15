# media-plane 0.12.0

_Released 2026-11-15._

### Dependencies
- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).

### Changed
- BREAKING: durations are `Duration` instead of bare seconds/floats (part of #2731): `SegmenterConfig::target_duration_secs: f64` is `target_duration: Duration` (set with `with_target_duration(Duration)`), `file_source::PassRange::span_secs`/`frame_secs` are `span`/`frame: Duration`, `segmenter::LeaderDurations` maps a leader segment number to a `Duration`, `segmenter::subtitle::LeaderPart::duration_secs` is `duration: Duration`, and `segmenter::audio::AudioFollower::try_new` takes its target duration as a `Duration`. A `Duration` cannot be negative, NaN or infinite, so `SegmenterConfig`'s former non-finite/negative check is now a zero check. Migration: `.with_target_duration_secs(1.0)` -> `.with_target_duration(Duration::from_secs(1))`, `range.span_secs` -> `range.span` (a `Duration`).
- BREAKING: `SegmenterEvent::OutputFailed(String)` is `SegmenterEvent::OutputFailed(TsHlsRejection)` — a typed `TsHlsRejection::{Hevc, Av1}` with `name()`/`reason()` — instead of a formatted message, so a host branches on the rejected codec rather than its text (#2734).
- BREAKING: the subtitle follower's internal segment number is a `broadcast_hls::MediaSequence` (and its part index a `PartIndex`), matching the cuts a leader hands it; `LeaderDurations` keeps its `u32` key, since it holds a segment duration per leader segment for every follower, not a playlist number (#2699).
- BREAKING: `ProgramSegmenter::continue_after_reconnect` takes the media start as a `Duration` (it was `media_start_ns: u64`), `SegmenterConfig::part_target_ms: u32` is `part_target: Duration` (constructed with `with_part_target(Duration)`), `SiblingTrunks::build_audio_followers` takes that `Duration`, and `SegmenterEvent::PartTargetMs(u32)` is `SegmenterEvent::PartTarget(Duration)`. Migration: `continue_after_reconnect(ns)` becomes `continue_after_reconnect(Duration::from_nanos(ns))`, `.with_part_target_ms(250)` becomes `.with_part_target(Duration::from_millis(250))`, and a host matching `PartTargetMs(ms)` matches `PartTarget(d)`.
- BREAKING: `ByteMerge::new` returns `Result<Self, MergeConfigError>` instead of panicking; the three assertions it made are now the typed variants `MergeConfigError::ZeroSources`, `MergeConfigError::ZeroMaxQueued` and `MergeConfigError::FailoverOutOfRange`. Migration: `ByteMerge::new(policy, 2, 8)` becomes `ByteMerge::new(policy, 2, 8).expect("counts are valid")`; a config-file count is checked with `if n == 0 { .. }` or by matching the error (#2659).
- BREAKING: `ingress::ReconnectPolicy` is `broadcast_common::backoff::ReconnectPolicy` (`backoff` + `max_attempts: Option<NonZeroU32>`); its old `new(max_attempts: NonZeroU32)` is `ReconnectPolicy::default().with_max_attempts(nonzero)`, and a `DialSupervisor` reads the bound through `ReconnectPolicy::exhausted` (#2690).
- BREAKING: `ByteTap::new` returns `Result<Self, TapError>` instead of panicking, and the failure is a typed `TapError::ZeroCapacity` (exposed as `media_plane::TapError`): a zero-capacity ring is reported rather than asserted, so the same call works from a configuration validator (#2733).

#### Fixed
- `epoch_ms_to_media` floors its media tick like `TimeAnchor::media_to_epoch_ms` now does instead of truncating toward zero, so a UTC instant just before a millisecond the forward direction floors onto no longer resolves to an inconsistent tick (issue #2724).
- Cold-ledger expiry adds the retention window with `checked_add`: an entry whose expiry does not fit the host clock stays in the ledger (it is never expired) rather than expiring at a saturated instant (#2685).

---

Published from tag `media-plane-v0.12.0`.
