# Changelog

All notable changes to `live-buffer` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Tests: Apple TV controller input-log goldens (`tests/apple_tv_input_logs.rs`): rai-3, france-5 and a stalling rai-3 log replayed verbatim through the current controller (private fixtures).

### Fixed
- Docs + test: caller contract for the start origin. The first reported end anchors the origin, so a caller must report
  the first end only from the join point (first video RAP) and withhold ends before it (audio reported ahead of the
  join point anchors start early and loses audio cushion). `on_media`/`on_track_media`/`register_track` rustdoc, README,
  DESIGN 0.1c; pinned by `audio_before_the_join_point_anchors_start_early`.
- Per-track silence timeout is now relative: a track is excluded only when its last delivery (or registration)
  is `track_silence_timeout_us` older than the latest delivery on ANY track. A global delivery gap (reconnect,
  server restart) excludes nobody, so the controller no longer Resumes on stale data and stalls again; the
  effective end stays the minimum of the tracks' last ends. The "all silent => latest end" fallback is removed.

### Added
- Per-track delivery-granularity floor: each track remembers the largest media-end step between delivery
  instants (max-hold, `granularity_hold_us`/`granularity_decay_us`, 30 s + 30 s) and the target is kept at least
  `largest step + margin` (capped by `max_us`, slew-limited when falling). Fixes audio that arrives in ~0.3 s
  bursts with low PDV (rai-3) stalling every ~10 s. Steps across a global data gap, faster-than-real-time
  catch-ups and forward jumps are ignored. New `ControllerConfig` fields `track_granularity`,
  `granularity_hold_us`, `granularity_decay_us` (+ `with_track_granularity`, `with_granularity_memory_us`,
  `ConfigField::GranularityDecay`), `Policy::need_with_granularity_us`. No ABI layout change.
- A starvation with data still arriving (`Reason::DataArriving`) is now charged to the stall memory (the failed
  target), so repeated track-limited stalls ratchet the target and the resume bar even when the PDV is low
  (before, only a PDV spike raised them). Behaviour change: such a `Pause` is now followed by a `TargetChanged`.
- `BufferController::unregister_track(track)`: remove a replaced/dead track from the per-track minimum so it
  cannot hold back the lead (unknown track: no-op). No ABI layout change.
- Docs: after a `PcrDiscontinuity` while playing the controller emits `Pause` then `Resume`, never a new
  `StartPlaying`; callers must treat both as "go" (README, DESIGN, `Event::Resume`). Pinned by a test.
- `StallMemorySnapshot::new` documented as the FFI/foreign-crate constructor (`const fn`; struct is `#[non_exhaustive]`).
- Per-track media ends: `BufferController::on_track_media(now, track, end)` / `register_track(track)`; the lead is
  measured against the minimum over the active tracks (a TS mux sends video ~1 s ahead and audio near
  real time; judging video alone made every audio buffer late). A track silent (relative to the others) for
  `ControllerConfig::track_silence_timeout_us` (default 3 s, validated non-zero,
  `ConfigField::TrackSilence`, `with_track_silence_timeout_us`) is excluded. `on_media` is documented as "end of media playable on ALL rendered tracks" and is track-0
  (`DEFAULT_TRACK`) shorthand. `Event::Pause::track` and `Stats::limiting_track` name the starving track;
  `EventRecord` gains `track`/`track_valid` (40 bytes), `ABI_VERSION` 1 -> 2. New `tracks` module, `MAX_TRACKS`.
- Initial crate (unpublished; no release approved). Sans-IO live-TS playout buffer controller:
  - `DeliveryJitter`: RFC 5481 PDV over a sliding-minimum window (with a long-gap rule so a
    stall longer than the window is still measured), burst-aware per-interval maxima, a Q15
    forgetting histogram (outlier split at 500 ms, the interval after a stall skipped once as
    burst aftermath), a stall max-hold (60 s hold + 60 s linear decay) and the RFC 3550 A.8
    jitter `J` (integer form) for reporting.
  - `Policy`: need = `max(k1 x baseline + margin, k2 x stall peak)`, asymmetric slew-limited
    follower, start and post-stall resume thresholds; `max_us` is a hard cap on all of them.
  - `BufferController`: Buffering / Playing / Starved state machine with explicit
    `StartPlaying` / `Pause` / `Resume` events, `Hint`s (`Reconnected`, `SourceEnded`, ...),
    PCR for the estimator and a separate PTS-domain media position (`on_media`) for the lead,
    PCR-less fallback, per-session `StallMemorySnapshot` (seed / snapshot / restore with aging).
  - C-ABI-friendly plain data: `ABI_VERSION`, stable `u32` codes, `#[repr(C)] EventRecord`,
    `Default` configs as the single source of defaults (initial target 1.8 s, owner-approved
    2026-10-17).
  - Examples `replay_trace` and `live_loop`; fuzz target `live_buffer`; synthetic generator and
    oracle tests; real-trace golden tests against `docs/live-buffer/analysis/golden_oracle.py`
    (private fixtures).
