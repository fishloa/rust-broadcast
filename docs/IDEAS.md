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

2. **"multimux" — just-in-time repackaging origin** (the Rust Shaka-Packager /
   Unified-Origin). SRT/RTSP/TS in → CMAF / HLS / LL-HLS / DASH / LL-DASH /
   TS-HLS per-request, **no transcode**. One tiny memory-safe binary or a WASM
   edge-worker replacing a Java/C++ packager. Crates: transmux + srt-runtime +
   rtsp-runtime + segmenters. The commercial centerpiece. Naming split:
   `transmux` stays the library (the demux→IR→mux engine); `multimux` is the
   product wrapping it — the live per-request HTTP origin server that actually
   serves traffic.

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

## New crates: SMPTE professional media-plant coverage

SMPTE opened its **entire standards catalog free** on 2026-06-17 (Standards, RPs,
Engineering Guidelines, RDDs — 800+ documents, permanently, at
`pub.smpte.org/doc/`). Confirmed independently across trade press + SMPTE's own
site. This unblocks a whole professional-broadcast-plant surface that was
previously paywalled, on top of the one SMPTE crate already shipped
(`smpte2038`, ANC data in MPEG-2 TS). Same filter as everything else in this
repo: parse the wrapper/protocol, never the bitstream. (Check each PDF's own
licence/redistribution terms before vendoring into `specs/` — the catalog page
itself doesn't state one.)

9. **[SHIPPED — st291 0.2.0, #648] ST 2110-40 — ancillary data over RTP.** The direct RTP sibling of the
   shipped `smpte2038` (ANC-in-TS): same ST 291-1 ANC packets, different
   transport. Built on free IETF **RFC 8331** ("RTP Payload for SMPTE ST 291-1
   Ancillary Data"), which alone is likely sufficient — there's even an
   existing open-source reference impl (`astronautlabs/rfc8331`) to
   cross-check against. Also the entry point into the wider **ST 2110** family
   (-10 timing, -20 uncompressed video/RFC 4175, -21 traffic shaping, -22
   JPEG XS, -30/-31 audio), all free now, the modern SDI-replacement
   professional-IP media transport. Top pick: smallest, most self-contained,
   most directly extends existing work.

10. **ST 337 — non-PCM audio in AES3.** Small (~15pp): the Pa/Pb/Pc/Pd
    preamble + burst-length framing that wraps compressed AC-3/E-AC-3/DTS
    bursts inside professional AES3 digital audio — wrapper-only, no
    bitstream decode. Directly extends the AC-3/E-AC-3/DTS codec-config work
    already shipped (dvb-si/transmux, MPEG-TS carriage) into the SDI/AES3
    professional-audio domain.

11. **ST 12-1 — Linear Timecode (LTC).** Small (~20pp), self-contained: the
    80-bit LTC frame + biphase-mark encoding. Pairs with ANC/VITC work
    (VITC/LTC commonly ride inside ANC packets) and the ST 2110-40 work above.
    Quick, low-risk addition.

12. **ST 377-1 — MXF core file format.** The strategic bet of this batch:
    KLV-based Partition/Header/Body/Footer structure + essence-container
    registry, ~130pp, free, self-contained without needing every operational-
    pattern sibling (OP-Atom, generic-container/AVC mappings, etc. — those can
    follow later, opaque-fallback in the spirit of issue #576). The one item
    here that opens a genuinely **new domain**: professional file-based
    interchange, vs. every other crate in this repo being live-stream-shaped.
    Bigger lift (M–L) than the others.

13. **RDD 6 / RDD 29 — Dolby E / Dolby Atmos metadata.** Niche but free,
    complements the existing Dolby (AC-3/E-AC-3) codec-config typing with the
    professional-audio-plant metadata layer (loudness/dialnorm/downmix,
    object-audio metadata) broadcast chains carry constantly over SDI.

Deprioritized: IMF (ST 2067 Composition/Packing List) and ST 2052-1
(SMPTE-TT/CFF-TT) — both XML-heavy, consistent with how TTML/IMSC got
deprioritized elsewhere in this project's spec surveys.

## Through-line
Ship **#1 (browser analyzer)** as the free on-ramp and **#2 (multimux, the JIT
origin)** as the product; 3–8 are features/verticals on the same core; 9–13 are
new-crate expansions into the professional-broadcast-plant domain, freshly
unblocked by SMPTE's catalog going free. All flow from the one architectural
bet: parse the wrappers, leave the bitstream opaque.

## Priority (value-for-effort, 2026-07-12)

All items now tracked as issues (#662-#673 below; #564/#614 are pre-existing
engineering issues, not from this list). Ranked by value delivered per unit of
new engineering — not by strategic importance alone, since the flagship item
(multimux) genuinely needs other work to mature first.

**Tier 0 — blocked, not an effort question.** #564 (CENC/CBCS) needs an
owner spec-sourcing decision (buy ISO/IEC 23001-7 vs. GPAC/Bento4-as-
ground-truth) before it can enter the effort ladder at all.

**Tier 1 — cheap, do now.**
- #614 — DASH golden-gate CI flake (mechanical, clears noise).
- #670 — ST 337 AES3 wrapper (~15pp, extends shipped AC-3/E-AC-3 typing).
- #671 — ST 12-1 LTC timecode (~20pp, pairs with st291's ANC work).
- #662 — Browser analyzer (WASM). A live WASM demo already exists for this
  workspace, so this is mostly UI/wiring on parsers that already ship
  (dvb-si/mpeg-ts/dvb-conformance/transmux/cc-data), not new spec work —
  best ROI of any product item.

**Tier 2 — solid value, more real work.**
- #665 — Compliance probe (media-doctor/dvb-conformance/streaming-demux
  exist; needs the always-on watch loop + Prometheus exporter).
- #666 — Captions-as-a-service (cc-data/dvb-subtitle/timed-metadata WebVTT
  shipped; needs the service wrapper).
- #673 — RDD 6/29 Dolby E/Atmos metadata (niche, extends shipped Dolby
  typing).
- #668 — ML demux front-end (thin wrapper + packaging; value depends on
  distribution more than engineering).

**Tier 3 — bigger commitment.**
- #672 — ST 377-1 MXF (biggest SMPTE lift, ~130pp, but opens a genuinely
  new domain: file-based interchange).
- #664 — SSAI ad-stitcher (real ad-tech value, more net-new splice/manifest
  logic than the Tier 2 verticals).

**Tier 4 — big bets, blocked on prerequisites maturing.**
- #667 — Embedded ingest gateway, #669 — on-camera LL-HLS origin (both need
  real hardware access + a new no_std HTTP server).
- #663 — multimux, the actual commercial centerpiece — needs
  `srt-runtime`/`rtsp-runtime` production-mature first. North star, not
  next.
