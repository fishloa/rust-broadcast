# buffering-runtime 0.4.0

_Released 2026-11-15._

Minor-class release (0.3 -> 0.4). The controller's public time surface is the workspace's shared types: a span is a `core::time::Duration`, a playback (PTS) position is a validated `broadcast_common::clock33::Clock27`, and a logged instant is the microsecond it is in rather than one rounded up, so a replayed log no longer runs a microsecond in the future.

Version: 0.3.0 -> 0.4.0.

## Breaking changes and migration

### Spans are `Duration`, media positions are `Clock27`

Every span (a config field, an `on_chunk`/`on_media`/`on_track_media`/`on_tick`/`on_hint` argument, an `Event`/`Stats`/`JitterEstimate`/`StallMemorySnapshot`/`Input` field, and the `Policy`/`MinWindow`/`DeliveryJitter` parameters) is a `core::time::Duration`, and every playback position is a `broadcast_common::clock33::Clock27`. The `Micros` and `Ticks27` newtypes are gone, so the reading-vs-media distinction now comes from `Duration` vs `Clock27`. The `_us`/`_27mhz` suffixes went with them where the type already says it: `policy.initial_us` is `policy.initial`, `Event::StartPlaying::lead_us` is `lead`, `Pause::depth_us`/`resume_target_us` are `depth`/`resume_target`, `TargetChanged::old_us`/`new_us` are `old`/`new`, `OverMax::need_us`/`max_us` are `need`/`max`, `ControllerConfig`'s `starve_threshold_us`/`nodata_after_us`/`track_silence_timeout_us` are `starve_threshold`/`nodata_after`/`track_silence_timeout`, `Controller::target_us()` is `target()`, `Policy::target_us`/`start_us`/`resume_us`/`max_us` are `target`/`start`/`resume`/`max`, and `JitterEstimate::{pdv_last_us, baseline_us, stall_peak_us, rfc3550_jitter_us}` are `{pdv_last, baseline, stall_peak, rfc3550_jitter}`. `InputLog::replay` returns `(Duration, Event)`.

```rust
// before
controller.on_chunk(arrival_us, bytes, Some(pcr_27mhz));
let target_us = controller.target_us();
let policy = PolicyConfig { initial_us: 300_000, .. };
// after
controller.on_chunk(Duration::from_micros(arrival_us), bytes, Some(Clock27::from_27mhz(pcr_27mhz)?));
let target = controller.target();
let policy = PolicyConfig { initial: Duration::from_micros(300_000), .. };
```

### `pts90k_to_27mhz` is `pts90k_to_clock27`

`pts90k_to_27mhz(pts)` is `pts90k_to_clock27(pts) -> Result<Clock27, ClockOutOfRange>`, so an off-clock PTS is reported rather than converted.

```rust
// before
let ticks = pts90k_to_27mhz(pts);
// after
let position = pts90k_to_clock27(pts)?;
```

### The input-log config row drops the unit suffix

A span's `config` row token is `jitter.min_window`, `policy.initial`, `starve_threshold`, `policy.max_step_down_per_s`, and so on. A log recorded with the old `_us` tokens still parses: `ConfigKey::from_name_or_legacy` accepts both, so an archived log replays unchanged.

```text
// before
config policy.initial_us 300000
// after
config policy.initial 300000
```

### `EventRecord` is `#[non_exhaustive]`

It is built by `Event::to_record` and field-assigned, never by an external struct literal.

## Fixes

- `time::duration_micros` (which rounded every span *up*) is split by what the span is, so a reading and a difference of two are truncated to the microsecond they are in and only a configured window still rounds up: `time::instant_micros` (a clock instant or a difference, floored; `time::duration_between` replaces the `ceil(a) - ceil(b)` `time::duration_sub`) and `time::window_micros` (a configured window, rounded up so a sub-microsecond setting is one microsecond, never a zero-length window). The maximum a difference of two floorings can understate is the new `time::MAX_DIFFERENCE_ERROR` (999 ns).

## Added

- The `time` module (`duration_micros`, `micros_as_duration`, `duration_add`, `duration_between`, re-exporting `Clock27`) for the boundary between a span and the whole microseconds the engine computes in, with `Clock27` serializing as its 27 MHz tick count.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `buffering-runtime-v0.4.0`.
