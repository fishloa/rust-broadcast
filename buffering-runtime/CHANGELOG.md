# Changelog

All notable changes to `buffering-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- A sparse-but-regular track (a video-only still-image slate: one picture every 2.29 s) is no longer charged the quantisation of its own cadence by the track silence rule. The silence clock was the arrival time of the latest delivery, so a registered track that never delivers (audio of a programme that switched to a video-only slate) was excluded only at the first delivery landing the silence timeout (3 s) after the first one, up to a whole step late: the real slate log resumed 6.876 s after the discontinuity, now 5.303 s (3 s after the first slate delivery). The clock now runs past the last delivery by at most the largest held delivery step (the time the next delivery is expected), so a global delivery gap still excludes nobody. The 0.7b start gate was not the cause (DESIGN 0.7d). A caller that knows the new timeline has no audio should `unregister_track` it at the discontinuity (the slate then resumes at its first delivery, 2.288 s).

### Tests
- `tests/slate_start.rs`: slate log golden (resume 5.303 s, was 6.876 s; mutation-proven), the unregistered-audio variant (2.288 s), a fixture-free synthetic of the same shape, and the rai-gulp 600 s clean baseline golden (one `StartPlaying` at 0.597 s, 0 pauses, final target 412 ms).

## [0.1.0] - 2026-10-18

Initial release. Sans-IO live-TS playout buffer controller for players that cannot change playback rate. The measurement half follows RFC 5481 / RFC 3550; the policy half (sizing, thresholds, stall memory) is **implementation-defined**: no standard specifies a de-jitter policy (owner-approved spec-posture departure).

### Added
- `DeliveryJitter`: RFC 5481 PDV over a sliding-minimum window (with a long-gap rule so a stall longer than the window is still measured), burst-aware per-interval maxima, a Q15 forgetting histogram (outlier split at 500 ms, the interval after a stall skipped once as burst aftermath), a stall max-hold (60 s hold + 60 s linear decay) and the RFC 3550 A.8 jitter `J` (integer form) for reporting.
- `Policy`: need = `max(k1 x baseline + margin, k2 x stall peak)`, asymmetric slew-limited follower, start and post-stall resume thresholds (`after_rebuffer` x1.25 of the stall depth); `max_us` is a hard cap on all of them.
- `BufferController`: Buffering / Playing / Starved state machine with explicit `StartPlaying` / `Pause` / `Resume` events, `Hint`s (`Reconnected`, `SourceEnded`, ...), PCR for the estimator only and a separate PTS-domain media position for the lead, PCR-less fallback, per-session `StallMemorySnapshot` (seed / snapshot / restore with aging; `StallMemorySnapshot::new` is the `const fn` FFI constructor). After a `PcrDiscontinuity` while playing the controller emits `Pause` then `Resume`, never a new `StartPlaying`.
- Per-track media ends: `on_track_media(now, track, end)`, `register_track(track)`, `unregister_track(track)`; the lead is measured against the minimum over the active tracks (a TS mux sends video ~1 s ahead and audio near real time). A track silent relative to the others for `track_silence_timeout_us` (default 3 s, validated non-zero) is excluded; a global delivery gap excludes nobody. A track registered before any media starts its silence clock at the first delivery on any track, so registration order and time do not change behaviour. `on_media` is track-0 (`DEFAULT_TRACK`) shorthand. `Event::Pause::track` and `Stats::limiting_track` name the starving track. New `tracks` module, `MAX_TRACKS`.
- Per-track delivery-granularity floor: each track remembers the largest media-end step between delivery instants (max-hold, `granularity_hold_us` / `granularity_decay_us`, 30 s + 30 s) and the target is kept at least `largest step + margin` (capped by `max_us`, slew-limited when falling). The delivery instant is the chunk boundary: all `on_track_media` calls for a track between two `on_chunk` calls are one delivery (a caller that never calls `on_chunk` gets calls under `COALESCE_US`, 1 ms, coalesced), so reporting per delivered chunk or per demuxed frame gives the same floor. Steps across a global data gap, faster-than-real-time catch-ups and forward jumps are ignored. Config: `track_granularity`, `with_track_granularity`, `with_granularity_memory_us`, `ConfigField::GranularityDecay`, `Policy::need_with_granularity_us`.
- Start/resume granularity gate: a `Buffering -> Playing` transition waits until every delivering track has shown two closed delivery steps, so the floor is already in the target; bounded by `policy.start_us()` for a slow or one-shot track, off with `track_granularity = false`.
- A starvation with data still arriving (`Reason::DataArriving`) is charged to the stall memory (the failed target), so repeated track-limited stalls ratchet the target and the resume bar even when the PDV is low; such a `Pause` is followed by a `TargetChanged`.
- Plain-data C-ABI surface: `ABI_VERSION` (2), stable `u32` codes, `#[repr(C)] EventRecord` (40 bytes, with `track` / `track_valid`), and `Default` configs as the single source of defaults (initial target 1.8 s, floor 300 ms, cap 3 s).
- Documented caller contract: report the first media end only from the join point (first video random-access point) and register every rendered track up front.
- Examples `replay_trace`, `live_loop` and `multi_track` (all run with no arguments); fuzz target `buffering_runtime`; synthetic generator and oracle tests; real-trace goldens against `docs/buffering-runtime/analysis/golden_oracle.py` (eight delivery traces) and Apple TV controller input-log goldens (private fixtures; they skip loudly where the private repository is not checked out).
