# Changelog

All notable changes to `buffering-runtime` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- A track registered while `Playing` (an audio switch, a PMT update) no longer turns off starvation detection: while playing the effective end is taken over the tracks that have delivered, and the registered-but-silent rule gates only a start or resume. A global delivery stall in that window used to let the renderer clock run past the data with no `Pause` (DESIGN 0.7f).
- PCR-less mode feeds the jitter estimator one track's own end series (the first track to deliver; another takes over with a fresh baseline only once it is unregistered or silent), not the minimum over tracks. With a second track joining more than `discontinuity_us` behind, every chunk read as a discontinuity, rebaselined and dropped the unregistered tracks, and the stream never started (DESIGN 0.7f).
- A restored stall memory keeps its exact age when the controller's first clock reading is smaller than that age (a host clock starting at 0 per session): the anchor was `now - age` clamped at 0, so the peak was held and decayed up to `age` too late.

## [0.2.1] - 2026-11-08
### Added
- `input_log` module: record what a host fed a `BufferController` and replay it offline with one format and one implementation. `Input` (typed, `#[non_exhaustive]`, one variant per controller call) with `apply` (the single place a recorded input becomes a controller call), `write_row`/`parse_row` (symmetric, typed `InputLogError` carrying the row), and `InputLog` (`ControllerConfig` as one `config` row per field via `ConfigKey`, an optional stall-memory `seed` row, then the inputs) with `write`, `parse`, `parse_with_config` (logs recorded before `config` rows existed, such as the Apple TV goldens), `rebase`, `controller` and `replay`. `hint` rows carry the hint's `name()` token. New example `replay_input_log` replays a log from the command line.
### Tests
- `reregistered_track_start`: a track re-registered after `unregister_track` holds the start until it delivers, like a first registration; a tick between the unregister and the register releases the start (the documented rule). Goldens on the tf-1 tvOS logs of 2026-11-08: the field start came from a tick while audio was unregistered (lead 1.000 s); without the two churn rows the same log starts at +0.681 s with lead 0.688 s covered by audio; the passing log is unchanged (+1.734 s, lead 0.544 s). No behaviour change: the controller was correct for its inputs (DESIGN 0.7e).

## [0.2.0] - 2026-11-07
### Documented
- Start-fill cost of the granularity gate, measured (DESIGN 0.7d): on the Apple TV input logs (rai-3, rai-3 per-chunk, rai-3-stall, france-5; closed loop, tracks registered up front) the gate costs nothing at `initial_us` 500 ms and 1.8 s; the first frame is bound by the audio lead filling in one-PES lumps (lead 703 ms at the rai-3 start for a 500 ms fill) and by audio's first delivery arriving ~0.43-0.62 s after video's. A one-step gate would start 0.1-0.2 s earlier at 300 ms but stalls rai-3 and rai-3 per-chunk, so the two-step gate and its bound are unchanged. No behaviour change.

### Tests
- `start_gate_is_free_at_500ms_and_prevents_the_300ms_stall` (real logs at 300 ms / 500 ms / 1.8 s fill: exact start instants, zero pauses) and `gate_adds_no_start_delay_at_500ms_and_above` (closed-loop simulation, jittered and steady audio, registered and not). Mutation-proved: one learnt step stalls rai-3 at 300 ms, three delays france-5 by 119 ms at 500 ms, dropping the `start_us()` bound delays the simulation start by 100 ms.
### Removed
- BREAKING: The `WRAP_27MHZ` re-export; use `broadcast_common::clock33::PCR_WRAP_27MHZ` (c2 TH8).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11. The `Cargo.toml` requirements name the new versions; no other manifest change.

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

### Fixed (found in integration soak before this release)
- A sparse-but-regular track (a video-only still-image slate: one picture every 2.29 s) is no longer charged the quantisation of its own cadence by the track silence rule. The silence clock was the arrival time of the latest delivery, so a registered track that never delivers (audio of a programme that switched to a video-only slate) was excluded only at the first delivery landing the silence timeout (3 s) after the first one, up to a whole step late: the real slate log resumed 6.876 s after the discontinuity, now 5.303 s (3 s after the first slate delivery). The clock now runs past the last delivery by at most the largest held delivery step (the time the next delivery is expected), so a global delivery gap still excludes nobody. The 0.7b start gate was not the cause (DESIGN 0.7d). A caller that knows the new timeline has no audio should `unregister_track` it at the discontinuity (the slate then resumes at its first delivery, 2.288 s).

### Tests (slate and baseline)
- `tests/slate_start.rs`: slate log golden (resume 5.303 s, was 6.876 s; mutation-proven), the unregistered-audio variant (2.288 s), a fixture-free synthetic of the same shape, and the rai-gulp 600 s clean baseline golden (one `StartPlaying` at 0.597 s, 0 pauses, final target 412 ms).

