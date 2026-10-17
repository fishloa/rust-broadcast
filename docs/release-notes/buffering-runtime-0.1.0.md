# buffering-runtime 0.1.0

_Released 2026-10-18._

Initial release. `buffering-runtime` is a sans-IO playout buffer controller for live MPEG-TS players that cannot change playback rate. It measures delivery jitter, remembers stalls, and emits explicit `StartPlaying`, `Pause` and `Resume` decisions plus a target buffer depth. It does no I/O, reads no clock, spawns no thread, demuxes nothing and uses no floating point; `PolicyConfig::max_us` is a hard cap on the target and on every start/resume threshold. `no_std` + `alloc`, depends only on `broadcast-common`, MSRV 1.95.0.

## Spec posture (read this first)

The **measurement half** follows published specifications: RFC 5481 packet delay variation (PDV) and the RFC 3550 A.8 interarrival jitter (reported only, never used for sizing). The **policy half** (the sizing rule, start/resume thresholds, stall memory, granularity floor) is **implementation-defined**: no standard specifies how deep a live-TS player buffer must be, so the crate cannot cite one. This is a deliberate, owner-approved departure from the workspace's spec-grounding discipline. Every default is tagged tuned / sourced / guess in `docs/buffering-runtime/DESIGN.md`, and the README's standards table marks the policy rows "Implementation-defined".

## What's new

- **Jitter measurement** (`DeliveryJitter`): RFC 5481 PDV over a sliding-minimum window with a long-gap rule (a stall longer than the window is still measured), burst-aware per-interval maxima, a Q15 forgetting histogram, a stall max-hold (60 s + 60 s decay) and the RFC 3550 A.8 jitter for reporting.
- **Policy and state machine** (`Policy`, `BufferController`): an asymmetric slew-limited target, a Buffering / Playing / Starved state machine with hysteresis, hints (`Reconnected`, `SourceEnded`, ...), a per-session `StallMemorySnapshot` (snapshot / restore with aging), a PTS-domain lead with PCR confined to the estimator, and a PCR-less fallback. After a PCR discontinuity while playing the controller emits `Pause` then `Resume`, never a new `StartPlaying`.
- **Per-track media ends**: `on_track_media`, `register_track` and `unregister_track`. The lead is measured against the **minimum over the active tracks**, because a TS mux sends video about 1 s ahead of play time and audio near real time; judging video alone made every audio buffer arrive late. A track silent relative to the others for `track_silence_timeout_us` (default 3 s) is excluded; a global delivery gap (reconnect, server restart) excludes nobody, so the controller never resumes on stale data. A track registered before any media starts its silence clock at the first delivery, so registration order and time do not change behaviour. `Event::Pause::track` and `Stats::limiting_track` name the starving track. `on_media` is track-0 shorthand.
- **Delivery-granularity floor**: each track remembers the largest media-end step between delivery instants (30 s hold + 30 s decay) and the target is kept at least `largest step + margin`. This fixes audio that arrives in coarse ~0.3 s bursts with a low PDV, which the PDV alone cannot see and which stalled every ~10 s. The delivery instant is the chunk boundary, so reporting once per delivered chunk or once per demuxed frame gives the same floor (calls under `COALESCE_US` of 1 ms apart are coalesced when the caller never calls `on_chunk`). Steps across a data gap, faster-than-real-time catch-ups and forward jumps are ignored.
- **Start/resume granularity gate**: a start or resume waits until every delivering track has shown two closed delivery steps, so the floor is already in the target the lead is compared with. Bounded by the start fill for a slow or one-shot track; off with `track_granularity = false`. Without it a start with `initial_us` below a track's PES step was granted blind and stalled within the first second.
- **`DataArriving` escalation**: a starvation while data is still arriving (`Reason::DataArriving`) is charged to the stall memory, so repeated track-limited stalls ratchet the target and the resume bar even when the PDV is low. The `Pause` is followed by a `TargetChanged`.
- **`after_rebuffer` 1.25**: the post-stall resume bar is `max(target, 1.25 x stall depth)` (was 1.5), trimming the extra freeze after a stall or burst. Measured on the real traces at `initial` 0.5 s: freeze after a stall about 0.35 s shorter; a same-size repeat stall is still survived, with less spare lead. The field stays configurable.
- **C-ABI v2**: plain-data surface for foreign callers: `ABI_VERSION = 2`, stable `u32` codes for states, reasons, hints and event kinds, and a `#[repr(C)]` `EventRecord` of **40 bytes** carrying `track` / `track_valid`. `StallMemorySnapshot::new` is a `const fn` constructor for FFI.
- **Defaults** (`ControllerConfig::DEFAULT`, also `Default`): initial target **1.8 s**, floor **300 ms**, cap **3 s**, resume at 1.25 x the stall depth, target fall limited to 100 ms/s, per-track silence timeout 3 s.
- **Examples**: `replay_trace`, `live_loop` and `multi_track`, each runnable with no arguments (`replay_trace` replays a built-in synthetic stall-and-burst trace).

## Caller contract

- **Join point.** The first media end the controller sees is its start origin. Report the first end only from the join point (the first video random-access point) and withhold ends for media before it: audio is muxed about 1 s behind video, and an audio end reported before the join point anchors the origin that much early and costs that much audio cushion. The controller cannot see random-access points.
- **Register up front.** Call `register_track` for every track you will render before the first delivery (at pipeline creation is supported; order does not matter). A registered track that has not yet reported blocks the start for up to `track_silence_timeout_us` counted from the first delivery on any track. A track announced after `StartPlaying` joins behind the playback position and costs a `Pause`.
- **Time domains.** `on_media` / `on_track_media` / `on_tick` use the playback (PTS) domain; PCR (via `on_chunk`) feeds the jitter estimator only, because PCR runs ahead of PTS by the mux/decoder delay and would overstate what can play.

## Validation

- **Public (runs everywhere, including CI):** a synthetic delivery generator with brute-force, exact-Q15, RFC 3550 A.8 and RFC 5481 oracles; property tests; closed-loop start/granularity/registration tests; a lexical no-float guard, the clock33 guard, the label-coverage drift guard and the `#[non_exhaustive]` coverage guard.
- **Private fixtures (not redistributable, held in the private fixtures repository; CI checks them out with a read-only deploy key and treats a missing fixture as a failure):** eight real delivery traces from a live TV head-end, replayed through the controller and compared against an independent stdlib-Python implementation (`docs/buffering-runtime/analysis/golden_oracle.py`, in the repository), plus Apple TV controller input logs (clean and stalling) replayed verbatim as goldens, including a per-chunk reporting variant. A source checkout without the private repository skips these tests loudly rather than passing silently. The published crate's own tests that do not need them run anywhere.
- The analysis behind the defaults is in `docs/buffering-runtime/DESIGN.md` and `docs/buffering-runtime/TRACE-ANALYSIS.md`.

## Limits

- The policy defaults come from eight real traces with three stall events; the cause of the ~1.0-1.5 s stalls is undetermined (transport retransmission or a producer-side stall) and the controller handles them from any cause. Total frozen time over a session is conserved by any buffer policy: a deeper start buffer only moves the first stall's cost from an unpredictable freeze into a predictable start delay.
- The controller cannot make a lossy link lossless and cannot change playback rate; the only way to rebuild a buffer is to pause.
- The actual lead can exceed `max_us` if the source delivers faster than real time; size queues as `max_us` plus a burst allowance.

## Dependencies

`broadcast-common` 10.0 (no default features; live at 10.0.0). Optional: `serde`. Dev-only (not published): `test-fixtures` (path), `serde_json`.

## Migration

New crate; nothing to migrate. `buffering-runtime = "0.1"`.
