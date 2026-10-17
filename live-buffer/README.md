# live-buffer

Sans-IO playout buffer controller for live MPEG-TS players: it measures delivery jitter, remembers stalls, and tells the caller **when to start, pause and resume** and how deep the buffer has to be. The policy half is **implementation-defined** (no standard specifies it); the measurement half follows RFC 5481 and RFC 3550.

[![crates.io](https://img.shields.io/crates/v/live-buffer.svg)](https://crates.io/crates/live-buffer)
[![docs.rs](https://docs.rs/live-buffer/badge.svg)](https://docs.rs/live-buffer)
![licence](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)
![MSRV](https://img.shields.io/badge/MSRV-1.95-orange)

> Status: **unpublished** (0.1.0 in the workspace, no release approved). The default numbers come from eight real delivery traces; see the design and trace analysis linked below.

## What it does / what it does not

It turns three inputs into decisions:

1. `on_chunk(arrival_us, bytes, pcr_27mhz)`: a chunk arrived, with its last PCR if any (feeds the jitter estimator only).
2. `on_media(now_us, media_end_27mhz)`: the demuxer's media position, **in the playback (PTS) domain** (end of the last demuxed sample).
3. `on_tick(now_us, playback_27mhz)`: the renderer clock, also PTS-domain.

and emits `Event`s: `StartPlaying`, `Pause`, `Resume`, `TargetChanged`, `OverMax`, `Rebaselined`. The lead is `media_end - playback`; PCR is never used for it, because PCR runs ahead of PTS by the mux/decoder delay and would overstate what can play.

It does **not**: do any I/O, read a clock, spawn a thread, demux, depend on `transmux`/`mpeg-ts` (PCR arrives as an integer), control playback rate (the only way to rebuild a buffer is to pause), switch variants, or use floating point. It cannot make a lossy link lossless: the total frozen time over a session is conserved (about 1.4 s per delivery gap, whatever the target); a deeper start buffer only moves the first gap's cost from an unpredictable freeze to a predictable start delay.

**Guarantee.** `PolicyConfig::max_us` is a hard cap on the target and on every start/resume threshold: the controller never asks the caller to hold more than `max_us` of media (`OverMax` reports when the estimated need was clamped). The actual lead can exceed it if the source delivers faster than real time; size queues as `max_us` plus a burst allowance.

## Standards and sources

Nothing here is a wire format, so `Parse`/`Serialize` do not apply. The columns follow `docs/DOC-STANDARD.md`.

| Source | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| RFC 3550, RTP | July 2003 | s6.4.1 and A.8 interarrival jitter `J` (integer form, x16 scaled) | Reference | reported as `rfc3550_jitter_us`, never used for sizing; tested against the A.8 algorithm |
| RFC 5481, Packet Delay Variation Metrics | March 2009 | s4.2 / s7.1.2 PDV = `D - Dmin` | Reference | the sample definition of the estimator |
| RFC 6298, TCP RTO | June 2011 | 1 s RTO floor, exponential back-off | Reference | explains the 1.0/2.0/4.0 s gaps used in the synthetic suite; no value is encoded from it |
| RFC 9002 s5, QUIC loss detection (RTT estimation) | May 2021 | minimum-filter idea | Reference | idea only |
| ExoPlayer/Media3, hls.js, Shaka (Apache-2.0), dash.js, libwebrtc (BSD-3) | n/a | live-offset adaptation, 60 s stability window, NetEq 500 ms interval maxima and forgetting histogram, `VCMTiming` 100 ms/s slew | Not implemented as code | ideas only, no code taken; GStreamer (LGPL) idea only |
| Queue/target heuristics (the policy) | n/a | start/resume thresholds, stall max-hold, hysteresis | **Implementation-defined** | every default is tagged tuned / sourced / guess in the design |

## Defaults

All in `ControllerConfig::DEFAULT` (also `Default`): initial target **1.8 s** (owner-approved 2026-10-17), floor 300 ms, cap 3 s, 2 s sliding minimum, 500 ms interval maxima, 10 ms x 400 histogram, q99, forgetting 0.98, 500 ms outlier split, 60 s hold + 60 s decay, resume at 1.5 x the stall depth, target fall limited to 100 ms/s, starvation at 20 ms.

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library; without it the crate is `no_std` + `alloc` (CI builds `thumbv7em-none-eabi`). |
| `serde` | no | `Serialize` on the public data types; `Deserialize` on the configs and `StallMemorySnapshot`. |

## Quick start

```toml
[dependencies]
live-buffer = "0.1"
```

```rust
use live_buffer::{BufferController, ControllerConfig, Event, pts90k_to_27mhz};

let mut c = BufferController::new(ControllerConfig::default(), None).unwrap();
let mut now = 0u64; // microseconds, your monotonic clock
let mut pcr = 1_000_000u64; // 27 MHz, from the TS
let mut pts = 90_000u64; // 90 kHz, from the demuxed video
let mut started = false;
while !started {
    now += 35_000;
    pcr += 35_000 * 27;
    pts += 35 * 90;
    c.on_chunk(now, 1316, Some(pcr));
    c.on_media(now, pts90k_to_27mhz(pts));
    c.on_tick(now, None); // renderer not running yet
    while let Some(ev) = c.poll_event() {
        if let Event::StartPlaying { media_27mhz, .. } = ev {
            // set the renderer clock to media_27mhz and rate 1
            let _ = media_27mhz;
            started = true;
        }
    }
}
assert!(c.target_us() <= ControllerConfig::default().policy.max_us);
```

## Caller contract (decision protocol)

- `StartPlaying { media_27mhz }`: set the renderer clock to it and rate 1.
- `Pause { hold_at_27mhz }`: **immediately** set rate 0 and hold the clock there; keep queueing every chunk that arrives while paused (it is the refill, never drop or render it). Tick at <= 50 ms; while paused the controller uses the held position, not yours.
- `Resume { media_27mhz }`: re-seed the clock to exactly this position and set rate 1 in one step. Rate 1 without the time re-creates the measured permanent starvation (a free-running clock is already ahead of the late data). After a discontinuity this differs from the pause position: it is a time jump on the caller's side, and if the audio renderer flushes on a time jump, re-enqueue from there.
- Send `Hint::SourceEnded`/`SourceError` when a connection drops and `Hint::Reconnected` **before** feeding the first chunk of the new one (re-baselines, keeps the stall memory, resumes at the target, not the post-stall threshold).
- Persist `snapshot_memory()` for the app session; on the next zap call `StallMemorySnapshot::aged(elapsed_us)` and pass it as the seed.
- C ABI: `ABI_VERSION`, `Hint::code`/`from_code`, `State::code`, `Reason::code`, `Event::to_record` (a `#[repr(C)]` `EventRecord`, field table in the rustdoc), `ControllerConfig::default()`; see section 7 of the design.

## Design notes

- Integer-only (Q8/Q15 fixed point, `u128` only inside one widening helper); no `f32`/`f64` in `src` (guarded by a test).
- Bounded memory: the sliding-minimum deque is pre-sized to `max_samples`, the histogram to `buckets`, the event queue to 16 (the oldest non-critical event is dropped first; `Pause`/`Resume`/`StartPlaying` survive unless the queue holds only those).
- PCR wrap arithmetic only through `broadcast_common::clock33` (a `clock33_guard` test bans `abs_diff` and a literal 2^33).
- Public enums carry `name()` and `Display` (workspace label convention; `tests/label_coverage.rs`).
- No panics on any input: `u64` extremes, non-monotonic clocks and zero config values return typed errors or are ignored and counted.
- Oracles: a brute-force sliding window, an exact Q15 reference, the RFC 3550 A.8 algorithm, the RFC 5481 definition and the stdlib Python scripts in `docs/live-buffer/analysis/` (an independent second implementation) against the real traces. The traces live in the private fixtures repo; the public suite carries a synthetic generator (RFC 6298 gap patterns, loss, ramping jitter, drift, reconnect).

Full design and evidence: [`docs/live-buffer/DESIGN.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/live-buffer/DESIGN.md), [`TRACE-ANALYSIS.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/live-buffer/TRACE-ANALYSIS.md).

## Related crates

`broadcast-common` (clock33 PCR arithmetic, the label macro) is the only dependency. A caller typically feeds it from `transmux` / `mpeg-ts` output, but this crate depends on neither. See `docs/CRATES.md`.

## MSRV and licence

MSRV 1.95.0. MIT OR Apache-2.0.
