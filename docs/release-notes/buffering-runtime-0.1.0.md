---
status: DRAFT
---

# buffering-runtime 0.1.0 (DRAFT, not approved for release)

**Release date:** not scheduled. The crate is unpublished and no tag exists; the owner has not approved a release. This note is a draft kept so the release lane (`release-buffering-runtime.yml`) has its required note.

Initial release. `buffering-runtime` is a sans-IO playout buffer controller for live MPEG-TS players that cannot change playback rate: it measures delivery jitter, remembers stalls, and emits explicit `StartPlaying`, `Pause` and `Resume` decisions plus a target buffer depth.

## What's new

- `DeliveryJitter`: RFC 5481 PDV (sliding-minimum window with a long-gap rule), burst-aware per-interval maxima, a Q15 forgetting histogram, a stall max-hold, and the RFC 3550 A.8 jitter for reporting.
- `Policy` and `BufferController`: asymmetric slew-limited target, Buffering/Playing/Starved state machine with hysteresis, hints (reconnect, source end), per-session stall memory snapshot/restore, PTS-domain lead with PCR confined to the estimator, PCR-less fallback.
- `max_us` is a hard cap on the target and on every start/resume threshold.
- Plain-data C-ABI surface (`ABI_VERSION`, stable codes, `EventRecord`) and `Default` configs (initial target 1.8 s).
- Two examples (`replay_trace`, `live_loop`), a fuzz target, and an oracle-backed test suite.

## Why this version

- **Proposed version:** 0.1.0 (new crate).
- **Status of the policy:** implementation-defined (no standard specifies a de-jitter policy); the spec-posture departure is awaiting owner approval.

## Dependencies

`broadcast-common` 10.0 (no default features). Dev-only: `test-fixtures` (path), `serde_json`.

## Limits

The policy defaults come from eight real traces with three stall events; the cause of the ~1.0-1.5 s stalls is undetermined (transport retransmission or a producer-side stall) and the controller handles them from any cause. Total frozen time is conserved by any buffer policy; a deeper start buffer only moves the first stall's cost into the start delay.
