# multimux — Live RTSP → LL-HLS/DASH JIT Origin (Design)

> Design spec for issue #663. Approved 2026-07-14. Companion track: #669 (Axis
> ACAP on-camera origin) is a separate, purpose-built crate — NOT a variant of
> this binary; it reuses only `transmux`, as the whole workspace does.

## Goal

A native (Linux/macOS) executable that is a thin **client + server wrap around
existing workspace crates**: pulls one or more live RTSP sources and repackages
each **just-in-time** into **LL-HLS** (EXT-X-PART), served from an in-process
tokio+axum HTTP origin. Muxing only — never transcodes, samples stay opaque.

## Scope (v1)

**In:**
- Ingest: RTSP **pull** (interleaved RTP/RTCP over TCP) via `rtsp-runtime`.
- Codecs: **H.264 video + AAC audio** first (H.265 only if cheap via the
  existing `hevc_config` path).
- Output: **LL-HLS only** (parts + blocking playlist reload), from an in-RAM
  CMAF store. A single **global output protocol** per instance ("same protocol
  out") — v1's only value is LL-HLS.
- **Routing:** N `input → stream-name` **routes**; multiple sources in,
  multiple served streams out, one uniform output protocol.
- Config: **CLI-first** (clap, workspace CLI standard `docs/CLI-STANDARD.md`)
  with an **optional JSON config file** (routes, segment/part durations,
  window, bind addr). CLI flags cover a single-route quick start; the JSON file
  is the multi-route form.
- Serving: standalone tokio+axum listener; RAM rolling window per stream
  (N segments + live parts), evict oldest on roll. Live-only.

**Out (deferred, explicitly):** **DASH + LL-DASH (v1.1** — the transmux DASH
surface is ready, it's just not v1's protocol); SRT/TS/file inputs; per-viewer
sessions / SSAI / manifest rewrites; DVR / VOD / disk spill; TLS + auth
(operator runs a reverse proxy in front); trick-play.

## Principle: libraries own the specs, multimux is glue

multimux wraps already-authored library crates. Any missing RTP/RTSP/SDP/codec
capability is a **library gap fixed upstream**, never carried in the app. So
multimux contains **no media/transport-spec logic** — only: config, the RAM
`StreamStore`, the axum origin (incl. LL-HLS blocking-reload), and the
per-source task that wires upstream pieces together.

### Upstream prerequisites (land + release BEFORE multimux consumes them)

Two `transmux` gaps must be closed first, each its own issue/story/PR, then a
`transmux` minor release; multimux then depends on that new version.

- **P1 — real streaming RTP demux input (`transmux`).** The current
  `rtp::RtpDepacketiser` is a batch test-helper: config-lossy (placeholder
  empty SPS/PPS), timing-lossy (`duration:0`, `is_sync:true`,
  `start_decode_time:0`). transmux is the demux hub ("demux any input into the
  neutral IR"); RTP is just another transport input alongside TS/fMP4/PS/WebM/
  FLV/RTMP. Upgrade to a **streaming, timing-aware, config-aware** depayloader
  (RFC 6184 H.264 FU-A/STAP-A + RFC 3640 AAC-hbr): real per-sample `duration`
  and `composition_offset` derived from RTP timestamps, real injected
  `CodecConfig`. Additive (new type/mode; keep the old helper) → minor bump.
- **P2 — SDP fmtp → `CodecConfig` (`transmux`).** transmux owns
  `AVCConfigurationBox`/`EsdsBox`, so it owns turning H.264
  `sprop-parameter-sets` (base64 SPS/PPS) and AAC `config=` fmtp into a
  `CodecConfig`. rtsp-runtime / `sdp-types` supplies the parsed fmtp; transmux
  converts. Additive helper → same minor bump as P1.

(If, while implementing, rtsp-runtime itself is found lacking any RTSP/RTP
transport detail, that too is fixed in rtsp-runtime upstream — same rule.)

### Existing surfaces multimux consumes (verified 2026-07-14)

| Need | Existing surface |
| --- | --- |
| RTSP client | `rtsp-runtime::io::AsyncRtspClient<S>` (feature `tokio`): `connect`, `describe`, `setup(uri,&Transport)`, `play`, `recv_interleaved()`; media as `ClientEvent::MediaData{channel:u8,data:Vec<u8>}` (opaque RTP; rtsp-runtime does not depayload) |
| RTP demux → IR | **P1's** new streaming transmux depayloader → `Media`/`Sample` |
| Codec config from SDP | **P2's** new transmux helper |
| Neutral IR | `media::{Media,Track}`, `pipeline::{Sample,TrackSpec,CodecConfig}` (`Sample` `#[non_exhaustive]` → `Sample::{new,from_annexb,from_raw}`) |
| LL-HLS segmenter | `ll_hls::LlHlsSegmenter::with_part_target(tracks, movie_timescale, target_duration_secs, part_target_ms)`; `init_segment()`, `push(track_id,Sample)`, `take_ready_parts()->Vec<PartInfo>`, `take_ready_segments()->Vec<SegmentInfo>`, `flush()` |
| LL-HLS playlist render | `hls::{MediaPlaylist, MediaSegment, PartSpec, LowLatencyConfig{part_target,part_hold_back,preload_hint_part}}`, `to_m3u8()` (emits `EXT-X-SERVER-CONTROL`/`PART-INF`/`PART`/`PRELOAD-HINT`) |
| DASH MPD (live/dynamic) | `dash::DashPackager` (`Package`): `dynamic`, `addressing:Addressing{Number,Timeline}`, `availability_start_time`, `publish_time`, `time_shift_buffer_depth`, `suggested_presentation_delay`, `media_template`, `start_number`; `package(&mut self,&Media)->Result<String>` |
| LL-DASH (v1.1) | `ll_dash::{LlDashPackager, LlSegmenter}` (already exported) |

`PartInfo{bytes,duration,independent,segment_seq,part_index}` /
`SegmentInfo{bytes,duration,segment_seq,part_count}` are the RAM-store units;
parts are `moof`+`mdat` (no `styp`), served as separate files. The segmenter
emits part bytes + metadata but assigns no URIs — multimux sets each
`MediaSegment.parts[].uri` when rendering the playlist.

## Architecture

One tokio task per source (ingest→segment), a shared RAM store per stream, an
axum origin reading the stores. `Sample` IR is the internal contract between
ingest and segmentation, exactly as elsewhere in the workspace.

```
RTSP source ──AsyncRtspClient──► ClientEvent::MediaData{channel,data}
    │  (DESCRIBE→SETUP→PLAY)              │
    │                                     ▼
    │                        per-channel RTP packet accumulator
    │                        (group into access units by marker bit)
    │                                     ▼
    │                        RtpDepacketiser → Media/Sample
    │                                     ▼
    │                        LlHlsSegmenter.push(track_id, Sample)
    │                                     ▼
    │              take_ready_parts()/take_ready_segments()
    │                                     ▼
    └───────────────────────────► RAM rolling window (StreamStore)
                                          ▲
              axum origin ────────────────┘  (read-only, per request)
```

### Modules (planned files, locked in the plan)

- `multimux/src/config.rs` — CLI (clap) + optional JSON config model + load;
  `routes: [{ input_rtsp_url, stream_name }]`, `target_duration_secs`,
  `part_target_ms`, `window_segments`, bind addr, output protocol (v1: LL-HLS).
  Precedence: JSON file if given, else CLI single-route quick start.
- `multimux/src/source/mod.rs` — `Source` trait: yields a `TrackSpec` set then a
  stream of `Sample` per track; lets #669's `VdoSource` slot in later, but v1
  ships only the RTSP impl. (Trait lives here; multimux is not forced to share
  a lib with #669 — each product crate owns its own ingest + HTTP glue.)
- `multimux/src/source/rtsp.rs` — `RtspSource`: drives `AsyncRtspClient`,
  reads DESCRIBE SDP, calls **P2** (transmux SDP→`CodecConfig`) for each media,
  groups interleaved RTP per channel and feeds **P1** (transmux streaming
  depayloader) → `Sample`s. No spec logic of its own — pure wiring.
- `multimux/src/store.rs` — `StreamStore`: per-stream RAM rolling window
  (init segment + `VecDeque<SegmentInfo>` + live `Vec<PartInfo>` for the
  in-progress segment); eviction; a `tokio::sync::watch` sender bumped on every
  new part/segment (the blocking-reload wakeup).
- `multimux/src/pipeline.rs` — the per-route task wiring `Source` →
  `LlHlsSegmenter` → `StreamStore`.
- `multimux/src/origin/mod.rs` — axum `Router` + shared `AppState` (map
  stream-name → `StreamStore`).
- `multimux/src/origin/hls.rs` — master + media `.m3u8`; **blocking reload**
  handling of `_HLS_msn`/`_HLS_part` query params (await the store's `watch`
  until the requested msn/part exists, then render); sets `PartSpec.uri` per
  part.
- `multimux/src/origin/segments.rs` — init / segment / part byte serving from
  the store (correct `Content-Type`, `Cache-Control`).
- `multimux/src/bin/multimux.rs` — `cli` feature-gated binary; clap.
- *(v1.1)* `multimux/src/origin/dash.rs` — `.mpd` via `DashPackager`; deferred.

### HTTP surface (v1)

- `GET /{stream}/master.m3u8`
- `GET /{stream}/media.m3u8[?_HLS_msn=&_HLS_part=]` — LL-HLS, blocking reload
- `GET /{stream}/init-{track}.mp4`
- `GET /{stream}/seg-{track}-{seq}.m4s`
- `GET /{stream}/part-{track}-{seq}.{part}.m4s`

*(v1.1 adds `GET /{stream}/manifest.mpd`.)*

## Data flow (one access unit)

1. `AsyncRtspClient` yields `MediaData{channel,data}` (one RTP packet).
2. Accumulator appends to the channel's current-AU buffer; on RTP **marker bit**
   (video) / AU boundary (AAC) the AU is complete.
3. Completed AU(s) → `RtpDepacketiser.unpackage(RtpInput{streams})` → `Media`
   with `Sample`s (length-prefixed NALs, `is_sync`, `composition_offset`,
   `SourceTiming{dts,pts}` from RTP timestamp).
4. `LlHlsSegmenter.push(track_id, sample)`.
5. Drain `take_ready_parts()` → append to store's live-part list, bump `watch`.
6. Drain `take_ready_segments()` → close segment into the window, evict oldest,
   clear live parts, bump `watch`.
7. axum requests read whatever the store currently holds; blocking-reload
   requests park on `watch` until their target lands.

## Error handling

- Structured `thiserror` errors (`ConfigError`, `SourceError`, `OriginError`),
  matching workspace conventions.
- A source task that errors (RTSP drop, malformed SDP, depayload failure) logs,
  marks its store **stale**, and retries connect with backoff; the origin serves
  a `503` for a stale/unstarted stream rather than a partial playlist.
- Blocking-reload requests have a bounded timeout (per the LL-HLS spec's
  hold-back); on timeout return the current playlist rather than hang forever.
- Unknown stream name → `404`.

## Testing / exit gate

Integration test (`multimux/tests/origin_llhls.rs`) — no network:
1. Drive a **loopback RTSP server** (rtsp-runtime's server state machine) that
   replies to DESCRIBE/SETUP/PLAY and emits interleaved RTP built from a
   committed H.264(+AAC) elementary-stream fixture.
2. Point a `RtspSource` at it; run the pipeline for enough AUs to close ≥2
   segments with parts.
3. Assert:
   - `media.m3u8` parses, contains `EXT-X-PART`, `EXT-X-PART-INF`,
     `EXT-X-SERVER-CONTROL` with sane `PART-HOLD-BACK`/`CAN-BLOCK-RELOAD`.
   - a blocking `?_HLS_msn=&_HLS_part=` request for a **not-yet-present** part
     resolves (does not 404) once the pipeline produces it.
   - init + a segment + a part each **parse as fMP4** (reuse `transmux`'s
     validator).
4. `#[ignore]` manual test against a public RTSP URL for real end-to-end.

Plus the workspace gate suite (build all-features + no-default-features, test,
clippy `-D warnings`, fmt, doc `-D warnings`) — multimux's HTTP/tokio code is
`std`+`cli`-gated; the crate must still `--no-default-features` build (lib core
compiles without the server, or the whole crate is gated — decided in the plan).

## New dependencies (multimux-only)

- `axum` (+ `hyper`, `tower`) — HTTP origin. New to the workspace, scoped to
  this crate.
- `tokio` (own pin, like every other crate — no `[workspace.dependencies]`).
- `rtsp-runtime` with its `tokio` feature; `transmux` (default features).
- `serde` + a config format (`toml`) for the config file.

MSRV 1.86 must hold with these added — verify the lockfile after adding
(axum/tokio minor pins can raise MSRV; pin down if so).

## Risks / open concerns

- **R1 — B-frame DTS reconstruction (in P1, upstream).** `Sample` has no
  explicit pts/dts: DTS = running Σ`duration`, PTS = DTS + `composition_offset`.
  RTP carries presentation time only. With B-frames, DTS≠PTS order and DTS must
  be reconstructed. **v1 assumes low-delay / no-B-frame H.264**
  (`composition_offset = 0`); P1 documents this limit and rejects/ignores
  reorder cases rather than mis-timing them. Full reorder support is later work.
- **R2 — RTP timestamp → track-timescale duration (in P1, upstream).** Per-AU
  `duration` = RTP-timestamp delta (video clock 90 kHz; AAC clock = sample
  rate). P1 must map the RTP clock to the `TrackSpec.timescale` cleanly.
- **R3 — DASH live from a rolling window (deferred to v1.1).** Not in v1
  (LL-HLS-only). When built: `DashPackager` already models dynamic/live MPDs
  (`dynamic`, `Addressing::{Number,Timeline}`, `availability_start_time`,
  `time_shift_buffer_depth`, templates); multimux builds a `Media` snapshot from
  the current window per request and sets those fields. No transmux change
  expected. Recorded here so the segment naming/window design stays
  DASH-compatible from day one.
- **R4 — RTP AU grouping in the origin path (multimux).** multimux still groups
  interleaved `MediaData` frames per channel and hands packet runs to P1's
  depayloader; confirm P1's streaming API shape (feed-per-packet vs feed-per-AU)
  so multimux's task loop matches it. Settled when P1's signature is fixed.

## Build order

**Phase 0 — upstream (separate issues + transmux minor release):**
0a. P1 — streaming RTP demux input in transmux (+ tests, real fixture).
0b. P2 — SDP fmtp → `CodecConfig` helper in transmux.
0c. transmux minor release; publish.

**Phase 1 — multimux (this crate, depends on the new transmux):**
1. Config (CLI + optional JSON, routes model) + CLI skeleton.
2. `Source` trait + `RtspSource` — drives `AsyncRtspClient`, groups RTP (R4),
   calls P1/P2; prove RTSP→Sample against the loopback fixture.
3. `StreamStore` + per-route pipeline task (segmenter wiring, RAM window,
   `watch`).
4. axum origin: segments → LL-HLS playlist → **blocking reload**.
5. Integration gate + docs/examples + RELEASE-DOCS.

*(v1.1: DASH output — `origin/dash.rs`, R3.)*
