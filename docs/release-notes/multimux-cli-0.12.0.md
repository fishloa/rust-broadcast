# multimux-cli 0.12.0

_Released 2026-10-31._

### Changed
- The single-route quick start now sets the new `multimux` route fields explicitly (`ts_hls_cut`, `ts_hls_idr_fallback`, `declared_bandwidth_bps`, `audio_renditions`, `audio_rendition_sources`, `closed_caption_overrides`, `audio_profiles`, `subtitles`) at their `multimux` defaults, so the CLI builds against the new route model. No new flag; no change for `--config` routes.
- Quick-start routes have `subtitles: true`, so DVB teletext subtitle pages are published as WebVTT renditions (`multimux` #1588).

---

Published from tag `multimux-cli-v0.12.0`.
