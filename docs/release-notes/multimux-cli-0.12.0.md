# multimux-cli 0.12.0

_Released 2026-10-31._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`multimux` 0.13 -> 0.14). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Changed
- The single-route quick start now sets the new `multimux` route fields explicitly (`ts_hls_cut`, `ts_hls_idr_fallback`, `declared_bandwidth_bps`, `audio_renditions`, `audio_rendition_sources`, `closed_caption_overrides`, `audio_profiles`, `subtitles`) at their `multimux` defaults, so the CLI builds against the new route model. No new flag; no change for `--config` routes.
- Quick-start routes have `subtitles: true`, so DVB teletext subtitle pages are published as WebVTT renditions (`multimux` #1588).

---

Published from tag `multimux-cli-v0.12.0`.
