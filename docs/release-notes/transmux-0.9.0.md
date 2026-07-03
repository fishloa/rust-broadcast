# transmux 0.9.0 — 2026-07-03

A large additive release that completes the **any-to-any container hub** and adds
IR-level transforms, conformance tooling, and a CLI. `transmux` still parses only
codec *config* headers — it never en/decodes; coded samples stay opaque.
`no_std` + `alloc`. Independently versioned.

## New demux/mux spokes

- **FLV** (#513) — `FlvDemux` / `FlvMux` ⇄ IR (Adobe FLV v10.1 Annex E; H.264 +
  AAC, ms timescale, lossless timing).
- **RTMP** (#515) — `RtmpDemux` / `RtmpMux` (Adobe RTMP 1.0: handshake,
  chunk-stream fmt 0–3 + reassembly, protocol-control, AMF0), routing A/V message
  bodies through the FLV spoke to the IR.
- **HEVC over MPEG-TS** (#467) — `TsDemux` now carries H.265 elementary streams
  into the IR (in-band VPS/SPS/PPS → `hvcC`), matching the fMP4 path. Also fixes
  a pre-existing `hvc1`/`hev1` visual-dimensions offset bug in `Fmp4Demux`.

## IR transforms

- **PTS/DTS rebase & timeline conditioning** (#476) — `Track::start_decode_time`
  anchor (populated by the TS/fMP4 demuxers, emitted by `CmafMux` as
  `tfdt`), plus `rebase_to_zero` / `apply_offset` / `unroll_33bit_wraps` /
  `insert_discontinuity_gap`.
- **Timeline splice / concat → SSAI** (#475) — `concat` / `splice_insert`
  (keyframe-snapped, byte-preserving, discontinuity-reporting) for server-side
  ad insertion.
- **I-frame trick-play track** (#477) — `derive_iframe_track` /
  `append_iframe_track`, plus HLS `EXT-X-I-FRAME-STREAM-INF` /
  `EXT-X-I-FRAMES-ONLY` and DASH `urn:mpeg:dash:trickmode:2016` manifest
  signalling.

## Packaging & conformance

- **HLS discontinuity** (#453) — `EXT-X-DISCONTINUITY` /
  `EXT-X-DISCONTINUITY-SEQUENCE`, explicit + auto-detected on init-segment change.
- **Low-Latency HLS** (#454) — `LlHlsSegmenter` partial segments + `EXT-X-PART` /
  `EXT-X-PRELOAD-HINT` / `EXT-X-PART-INF` / `EXT-X-SERVER-CONTROL` (RFC 8216bis).
- **`emsg` in the segmenter** (#455) — `build_media_segment_with_events` attaches
  ISO 14496-12 event-message boxes (in-band SCTE-35 / ID3), via `mp4-emsg`.
- **fMP4/CMAF conformance validator** (#481) — `validate_init_segment` /
  `validate_media_segment` / `validate_cmaf_track` → `ConformanceIssue`.
- **RTCP** (#514) — typed SR/RR/SDES/BYE/APP + `CompoundPacket` (RFC 3550 §6).
- **NAL keyframe helper** (#517) — `nal_unit_type` / `is_keyframe_nal` /
  `access_unit_is_keyframe` (AVC/HEVC/VVC).

## CLI & codec metadata

- **`transmux` CLI packager** (#482) — behind the opt-in `cli` feature: autodetect
  input container → IR → chosen output, `--segment-duration`/`--ll`/`--tracks`,
  `--decrypt` (with `cenc`). Library stays `no_std`/clap-free.
- **AVC VUI framerate** (#523) — `decode_avc_sps` now exposes
  `num_units_in_tick`/`time_scale`/`fps`.
- **HEVC SPS** (#516) — `decode_hevc_sps` verified against a real fixture.

## Compatibility

Requires broadcast-common ≥ 8.2. Minor-version additive; two pre-1.0 behavioural
refinements are flagged in the CHANGELOG (`AvcSpsInfo` dropped `Eq` for the new
`f32` `fps`; `CmafMux` now emits the track's real `tfdt` instead of a hardcoded
0 — a correctness improvement). MSRV unchanged (1.81).
