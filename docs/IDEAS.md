# Products to build on top of rust-broadcast

The stack's defining bet: **parse containers, protocols, and broadcast SI — never
the codec bitstream.** So everything below is memory-safe, ffmpeg-free, tiny,
`no_std`/embeddable where it counts, and WASM-able. That combination unlocks
products a transcode-bound stack can't ship. Not committed roadmap — a vision
backlog; promote any to an epic when ready.

## Flagship

1. **"Wireshark for broadcast" — 100% in the browser (WASM).**
   Drop a `.ts`/`.mp4`/`.mpd` → SI/PSI tree, PID map, PCR/PTS drift graphs,
   SCTE-35 splice timeline, TR 101 290 report, caption preview — all client-side,
   nothing uploaded. Enabled by `no_std` + no-transcode → runs in-browser.
   Free viral on-ramp. (skyfire is already partway here.) Crates: dvb-si,
   mpeg-ts, dvb-conformance, transmux, cc-data.

2. **Just-in-time repackaging origin** (the Rust Shaka-Packager / Unified-Origin).
   SRT/RTSP/TS in → CMAF / HLS / LL-HLS / DASH / LL-DASH / TS-HLS per-request,
   **no transcode**. One tiny memory-safe binary or a WASM edge-worker replacing
   a Java/C++ packager. Crates: transmux + srt-runtime + rtsp-runtime +
   segmenters. The commercial centerpiece.

## Verticals / features on the same core

3. **SSAI ad-stitcher.** SCTE-35 cues → splice ad CMAF → per-session manifest
   rewrites (HLS DATERANGE / DASH emsg). transmux splice/concat + scte35 +
   timed-metadata + mp4-emsg already do the hard parts. Ad-tech value.

4. **Live 24/7 compliance probe.** media-doctor + dvb-conformance + streaming
   demux → watch an SRT/UDP feed, continuously assert TR 101 290 + codec
   signalling-vs-bitstream + caption presence + SCTE-35 sanity → Prometheus/
   Grafana. A rack-probe-class monitor as a $20 appliance.

5. **Captions-as-a-service / archive rescue.** cc-data + timed-metadata WebVTT +
   dvb-subtitle + teletext → pull 608/708/DVB-sub/teletext from any source →
   WebVTT/IMSC/SRT. Accessibility mandates + archive digitization.

6. **Embedded ingest gateway (homelab-dogfoodable).** `no_std` core on a small
   box: SRT/RTSP in → ts-fix (PCR/CC repair) → re-mux → local HLS. Point at an
   hdhomerun + CCTV feeds → self-hosted ffmpeg-free TV/CCTV gateway; dvb-si SI →
   self-hosted EPG API.

7. **Demux front-end for ML video.** Clean, PTS-tagged, opaque samples straight
   to WebCodecs/ONNX (scene/ad/content-ID detection) with no ffmpeg glue.
   Pip/npm-able primitive.

8. **On-camera LL-HLS origin (firmware, no media server, no restream hop).**
   Runs **inside** the camera. The SoC's own H.264/H.265 encoder already emits an
   Annex-B NAL byte-stream (V4L2 M2M / vendor encoder API) — so you skip RTSP and
   RTP entirely: feed the encoder output straight into the NAL parser → neutral IR
   → `LlHlsSegmenter` (EXT-X-PART partials) → a tiny embedded HTTP server. The
   camera **is** the LL-HLS origin; a browser hits it directly for sub-second-ish
   viewing with NO MediaMTX/go2rtc, NO WebRTC gateway, NO transcode.
   Why this stack: it's the *only* one that fits an SoC — `no_std`+alloc, no
   ffmpeg, tiny footprint. **#595 open-GOP anchoring is exactly what cameras need**
   (they rarely emit IDR); **#599 SEI captions** picks up camera-embedded captions.
   Build pieces present: NAL/AU parsing (`transmux::nal`), IR, `LlHlsSegmenter`.
   Gaps to close: (a) Annex-B start-code framing → AU boundary input adapter
   (byte-stream, not RTP — `transmux::RtpDepacketizer` is for the *off-box*
   restream variant only), (b) a minimal `no_std`-friendly HTTP/1.1 server
   (picoserve-class — chunked partials + blocking playlist reload; NOT axum),
   (c) H.265 path for HEVC SoCs (parse/segment; H.264 is done),
   (d) aarch64/armv7 musl cross-compile + a small vendor-encoder shim.
   Off-box restream variant (Pi beside a legacy RTSP-only camera) reuses
   `rtsp-runtime` + `RtpDepacketizer` instead of the encoder adapter.
   Competes with go2rtc/MediaMTX but pure-Rust, firmware-sized, proper LL-HLS.

## Through-line
Ship **#1 (browser analyzer)** as the free on-ramp and **#2 (JIT origin)** as the
product; 3–8 are features/verticals on the same core. All flow from the one
architectural bet: parse the wrappers, leave the bitstream opaque.
