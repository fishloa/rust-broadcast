# transmux 0.15.0 — 2026-07-06

Additive release: live multi-track streaming ingest, HEVC TS carriage, an
on-camera Annex B ingest primitive, MPEG-H TS carriage, in-band caption
extraction, and an independent-decoder validation gate. No breaking changes.

## Added

- **Late-resolving live tracks** (#624) — `DemuxEvent::TracksResolved` (fires
  once every currently-known PMT-declared PID has resolved, safely re-arming
  on a later PMT bump) and `StreamingTsHlsSegmenter::add_track` (register a
  track after construction, with safe anchor promotion when nothing has been
  cut yet). Fixes the real-world case where a live TS's audio PID resolves
  after the first video keyframe, which previously left a consumer stuck
  building a permanently video-only segmenter.
- **Streaming Annex B → access-unit splitter** (`au::AccessUnitSplitter`,
  `au::split_access_units`, #601) — incremental AU-boundary detection over a
  raw Annex B byte stream (no TS/PES framing), for on-camera SoC-encoder
  ingest. Codec-aware (AVC/HEVC; VVC on AUD/non-VCL boundaries only), `no_std`,
  byte-exact.
- **HEVC / MPEG-2-video / MPEG-1/2-audio TS carriage** (#627) — `EsKind::Hevc`
  (stream_type `0x24`, with the same independently-decodable VPS/SPS/PPS
  keyframe-prepend guarantee AVC already had), `EsKind::Mpeg2Video` (`0x02`),
  `EsKind::MpegAudio` (`0x03`/`0x04`). These codecs were previously silently
  dropped from TS and TS-HLS output entirely.
- **Anchor-track selection recognises every video codec** (#628) —
  `CodecConfig::is_video()`; `choose_anchor`/`Segmenter::new` (and, after a
  pre-tag audit caught the gap, `add_track` too) now anchor segmentation on
  any video track, not just AVC.
- **MPEG-H 3D Audio TS carriage** (#579) — PMT `stream_type 0x2D` recognition,
  MHAS packet-framing walker, `CodecConfig::MpegH` track construction, and the
  `MPEG-H_3dAudio_descriptor` PMT synthesis. Config/sample passthrough only
  (no bitstream decode), matching the crate's AC-3/DTS carriage posture.
- **`nal::caption_cc_data`** (#599) — extract ATSC A/53 caption SEI
  (CEA-608/708) directly from an H.264/HEVC access unit, in the same wire
  form the PES-carried path already produces. Validated against a real
  captured ATSC A/53 SEI.
- **Player-validated golden gate** (#569) — `tests/golden_gate.rs` packages a
  real fixture through the actual `transmux` CLI code path into
  CMAF/progressive-MP4/TS-HLS/DASH, then validates each artefact with an
  independent decoder (`ffprobe`) rather than the crate's own parsers. New
  non-blocking `golden-gate` CI job.

## Fixed

- **Reverted a wrong #629 fix.** An earlier "fix" to
  `StreamingTsHlsSegmenter`'s `#EXT-X-DISCONTINUITY-SEQUENCE` bookkeeping was
  itself incorrect — a pre-tag audit re-derived RFC 8216 §6.2.1's literal
  definition and found the "fix" double-counted a discontinuous segment's
  boundary. The original eviction-based logic was correct all along and has
  been restored, with a regression test that computes each segment's true
  client-observable Discontinuity Sequence Number (not just the raw header
  integer) to prevent this class of error recurring.
- **`add_track`'s anchor-promotion was AVC-only**, missed by #628's broader
  `is_video()` fix — a late-resolving HEVC (or other non-AVC video) track
  added via `add_track` never got promoted to anchor. Fixed.

## Compatibility

No breaking changes — purely additive. MSRV 1.86, edition 2024. Workspace
dependents `media-doctor` and `timed-metadata` have their `transmux` path-dep
version requirement bumped to `0.15` (a local workspace-build requirement;
neither crate's own published behavior changed, so neither is re-released in
this wave).
