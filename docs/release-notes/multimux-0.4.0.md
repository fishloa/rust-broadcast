# multimux 0.4.0 — 2026-07-21

Minor. True chunked-transfer LL-DASH, replacing the discrete-parts-signalling fallback shipped in 0.3.0. Additive — existing `ll_hls`/`dash`/`ll_dash` outputs and config are unchanged.

## Changed — true chunked-transfer LL-DASH (#721)

The `ll_dash` output previously served a spec-legal but not-player-low-latency-validated MPD that re-addressed the LL-HLS `part-*.m4s` files as discrete DASH units (honest `availabilityTimeOffset="0"`). It now serves **true chunked-transfer LL-DASH**:

- **MPD** (`manifest-ll.mpd`) is rendered via `transmux::LlDashPackager` — a whole-segment `SegmentTemplate` (`seg-{track}-{seq}.m4s`, the same addressing `manifest.mpd` uses) with a genuinely non-zero `availabilityTimeOffset` + `availabilityTimeComplete="false"`, and a real `timeShiftBufferDepth` (closed segments stay in the window).
- **Segment delivery**: the shared origin resource route serves a not-yet-closed segment over **HTTP chunked transfer-encoding**, streaming the store's live parts (moof+mdat) as they land and ending the response when the segment closes. A closed segment is served whole (`Content-Length`) exactly as before. This uses the existing blocking-wait/notify machinery — no busy-polling — with an abuse bound so a wildly-future segment number 404s immediately.
- **Byte path**: the LL-HLS part bytes (bare moof+mdat) concatenate into a segment that Chromium/dash.js's MSE accepts directly, so no separate chunk-shaped segmenter was needed — the proven store bytes are reused.

## Validated against a real headless dash.js low-latency player

`multimux/tests/lldash_dashjs.rs` spins up a real multimux LL-DASH origin (fed the `h264_aac.ts` fixture through the real `LlHlsSegmenter`, live-paced at 25 fps) and drives **headless Chromium via Playwright** loading **dash.js 5.2.0** (vendored under `multimux/tests/assets/`, BSD-3-Clause) in low-latency mode. It asserts real playback (`video.currentTime` advances past the manifest-parse bar) with a measured live latency **below the 1 s segment target** (min ≈ 0.63–0.75 s across runs) and no fatal dash.js error — proving genuine chunked availability, not whole-segment delivery. The test skips cleanly when `node`/Playwright are absent.

## Compatibility

MSRV 1.86. No API or config changes — recompile. The `ll_dash` output token and `manifest-ll.mpd` path are unchanged; only the manifest content + segment-delivery transport improved.
