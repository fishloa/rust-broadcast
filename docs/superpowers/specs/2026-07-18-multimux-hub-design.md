# multimux → multi-input / multi-output JIT origin — design

**Goal:** Grow `multimux` from a single-purpose RTSP→LL-HLS repackager into the
`#663` **just-in-time origin hub**: ingest from many live transports, package
each ingested stream to many output protocols simultaneously, and be operable
unattended 24/7. Keep all protocol parse/mux logic in the libraries
(`transmux`, `rtsp-runtime`, …) — multimux is the client+server wrapper.

**Approved decisions (2026-07-18):**
- **Sequencing:** foundation-first — harden + generalize (P0–P2) before adding
  inputs/DASH (P3–P4).
- **Output model:** ingest-once → many outputs. One input feeds LL-HLS *and*
  DASH (and future outputs) from one shared neutral IR/store.
- **Cadence:** one long-lived branch `feature/multimux-hub`, per-phase commits
  as checkpoints, a single multimux release at the end.

## Architecture

```
             ┌── input adapters (transport + transmux demux) ──┐
 RTSP pull ──┤                                                 │
 UDP/RTP  ───┤   each implements  pipeline::SampleSource        │
 raw RTP+SDP─┤   → yields (track_id, Sample) batches            │
 TS/UDP   ───┤                                                 │
 TS/HTTP  ───┤                                                 │
 HLS pull ───┘                                                 │
                          │  Sample / TrackSpec (transmux IR)   │
                          ▼                                      │
                 run_pipeline → LlHlsSegmenter (+ DASH segmenter)│
                          │                                      │
                          ▼                                      │
                   MediaStore (neutral, per-track init/seg/part) │
                          │                                      │
         ┌────────────────┼───────────────────┐                 │
         ▼                ▼                    ▼                 │
   LL-HLS origin     DASH/LL-DASH origin   (future: TS-HLS,…)    │
   /{s}/*.m3u8       /{s}/*.mpd + segs                           │
```

### The Output abstraction (the core refactor)

Today `StreamStore` + `origin/handlers.rs` are LL-HLS-specific (m3u8 render,
part URIs, blocking `_HLS_*`). To serve multiple outputs from one ingest we
split responsibilities:

- **`MediaStore`** — protocol-neutral rolling window of the segmenter's output
  keyed by track: `init`, closed `segments`, live `parts`, `recent_parts`,
  plus the progress `watch`. It holds *bytes + timing*, not playlist/manifest
  syntax. (This is today's `StreamStore` minus the m3u8 rendering.)
- **`trait Output`** — one impl per output protocol. Given a `&MediaStore`,
  it renders its manifest (m3u8 / MPD) and resolves its media-segment/part
  URIs to bytes, and mounts its own axum sub-router under the stream path.
  - `LlHlsOutput` — today's handlers/rendering, moved behind the trait.
  - `DashOutput` — MPD + `LlDashPackager`/`DashPackager` (P4).
- **`origin`** — mounts, per route, each configured `Output`'s sub-router under
  `/{stream}/…`, sharing the one `MediaStore`.

The **segmenter feeds the store once**; each `Output` renders its own view.
LL-HLS and DASH share the same CMAF init/segments (both are fMP4/CMAF), so the
store's bytes are reused, not re-muxed per output.

### Input adapters

Each is a `SampleSource` = a transport source + an existing transmux demux:

| Input | Transport (new in multimux) | Demux (exists in transmux) |
|-------|-----------------------------|----------------------------|
| RTSP pull | `rtsp-runtime` (have) | `rtp_stream` depay (have) |
| raw RTP + SDP | UDP socket + out-of-band SDP | `rtp_stream` + `rtp_sdp` |
| UDP/RTP | UDP socket (uni/multicast) | `rtp_stream` |
| TS over UDP | UDP socket (uni/multicast) | `ts_demux::StreamingTsDemux` |
| TS over HTTP | `reqwest`/hyper GET stream | `ts_demux::StreamingTsDemux` |
| HLS pull | HTTP playlist+segment fetch | `ts_demux` / `Fmp4Demux` |

No new codec/container parsing — transmux owns it. Any gap found (e.g. a
depay edge, an SDP case) is fixed **upstream in transmux**, not in multimux.

## Phased plan (foundation-first, one branch)

Each phase = one or more commits on `feature/multimux-hub`, gates green before
moving on. Single release (multimux minor bump) after P6.

### P0 — Foundation & resilience (unblocks everything)
1. **RTSP auth wiring** — read URL userinfo + `with_credentials` (rtsp-runtime
   already does Basic/Digest). Password cameras must work. *(audit-ingest #2)*
2. **Reconnect + supervision** — a route runs a supervise loop: on source
   error/EOF, mark the stream **stale** (store carries a health state), retry
   with capped exponential backoff; never a permanent silent zombie.
   *(cross-cutting #1 — every audit)*
3. **Store health state + stale semantics** — `MediaStore` exposes
   `HealthState { Live, Stale{since}, Failed }`; outputs render accordingly
   (LL-HLS `#EXT-X-ENDLIST` on terminal, appropriate status on failed).
4. **Graceful shutdown** — ctrl-c/SIGTERM → `axum::with_graceful_shutdown` +
   drain in-flight blocking reloads + stop supervisors. *(audit-concurrency #2)*
5. **`MediaStore`/`Output` split** — extract the neutral store + the `Output`
   trait; move current LL-HLS behind `LlHlsOutput`. No behaviour change; all
   existing origin tests still pass. *(enables P4)*

### P1 — Observability
1. `tracing` throughout (structured spans per route/request; replace the 3
   `eprintln!`). Workspace-wide first use — add the dep. *(audit-ops #1)*
2. Prometheus `/metrics` (route up/down, source state, segment/part rate,
   active blocking clients, reconnect count, request latency).
3. `/healthz` + `/readyz`.
4. **Secret redaction** — never log RTSP `user:pass`; redact in `Debug` +
   error text. *(audit-ops #2)*
5. **Structured error enum** — replace `Config(String)`/`Source(String)` with
   field-carrying `thiserror` variants, `#[non_exhaustive]`, per workspace
   convention. *(audit-ops #3)*

### P2 — LL-HLS spec-conformance
1. `#EXT-X-TARGETDURATION` = **max actual** segment duration (track it), not
   the configured target. *(audit-llhls #1, a MUST-violation)*
2. Blocking reload: bare `_HLS_msn` waits for a **closed** segment, distinct
   from `_HLS_part=0`. *(audit-llhls #2)*
3. `Cache-Control` (parts/segments immutable; playlist no-cache) + permissive
   CORS. *(audit-llhls)*
4. `_HLS_msn`/`_HLS_part` abuse rules: 400 on far-future/malformed, bounded.

### P3 — Input transports (each a `SampleSource` + transmux demux)
1. **UDP/RTP** (uni + multicast) → `rtp_stream` depay.
2. **raw RTP + SDP** (SDP from config/file) → `rtp_sdp` + `rtp_stream`.
3. **TS over UDP** (multicast) → `StreamingTsDemux`.
4. **TS over HTTP** → `StreamingTsDemux`.
5. **HLS pull** → `ts_demux`/`Fmp4Demux`.
Each with reconnect (via P0 supervisor), timeouts, bounded buffering.

### P4 — DASH output
1. `DashOutput` behind the `Output` trait: MPD (`.mpd`) + init/segments from
   the shared CMAF store via transmux `DashPackager`.
2. LL-DASH: `LlDashPackager` — LL-DASH parts + `availabilityTimeOffset`,
   chunked-transfer semantics.
3. Config: `outputs: [llhls, dash, lldash]` per route, all off one ingest.

### P5 — Hardening
1. Ingest timeouts (connect/read) everywhere. *(audit-ingest #3)*
2. Bounded FU-A/AU reassembly + TS buffer caps (memory-DoS). *(audit-ingest)*
3. HTTP-layer limits: connection cap, idle timeout, body cap (slow-loris).
   *(audit-concurrency #3)*
4. RTCP SR for RTP→wallclock A/V sync. *(audit-ingest)*
5. Mutex-poisoning resilience (don't brick a route on a poisoned lock).

### P6 — Tests, fuzz, docs
1. Source-error + reconnect/backoff coverage (mock servers).
2. SDP parse **fuzz** target (untrusted wire bytes). *(audit-tests)*
3. DASH conformance (validate MPD + segments) + LL-HLS validator pass.
4. e2e: each input type → both outputs → playable bytes.
5. Config validation completeness; Dockerfile + systemd + deploy doc.
6. README/CHANGELOG; single multimux release.

## What's existing vs net-new vs upstream

- **Existing (transmux/rtsp-runtime):** all demux/mux (DASH, LL-DASH, TS, RTP,
  HLS, fMP4), RTP depay, RTSP client, LL-HLS segmenter. Reuse verbatim.
- **Net-new (multimux):** transport adapters (UDP/HTTP sockets), the
  `MediaStore`/`Output` split, `DashOutput`, supervisor/reconnect,
  observability, config for multi-in/out.
- **Upstream (transmux) if gaps surface:** any demux/depay/SDP edge — fix in
  transmux, release, consume. Never re-implement parsing in multimux.

## Success criteria
- One `multimux` instance ingests ≥2 input *types* concurrently and serves each
  as **both** LL-HLS and DASH, playable in a browser (hls.js + dash.js).
- A source drop auto-recovers (backoff) with the outage visible in
  metrics/logs — no silent zombie.
- `SIGTERM` drains cleanly. `/metrics` + `/healthz` present.
- LL-HLS validator + DASH conformance pass. All gates green. SDP fuzz runs.
- No secret ever logged. Structured errors. No new parsing in multimux.

## Non-goals (this effort)
- Transcoding / bitrate ladder (samples stay opaque; one rendition per input).
- Push ingest (RTMP/SRT server), WebRTC — later.
- Auth/DRM on the output side — later.
