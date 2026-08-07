# media-doctor 0.7.0

**Release date:** 2026-08-02

Adds an Apple `mediastreamvalidator` oracle test harness (issue #870) — a
genuinely independent HLS conformance check that caught a real gap in
`transmux`'s CMAF-HLS packager (missing `#EXT-X-MAP`). Also tightens
`hls-part-duration-range` to Error severity and migrates HLS playlist parsing
to the new `broadcast-hls` crate.

## What's new

- **`mediastreamvalidator` oracle** (issue #870): Apple's own
  `mediastreamvalidator` validates every HLS playlist shape the origin can
  produce. macOS-only; skips cleanly on Linux/CI.

## What changed

- `hls-part-duration-range` severity raised from Warning to **Error**
  (RFC 8216bis §4.4.4.9 — the 85% partial-segment duration floor is a MUST).
- `check_hls_playlist` now uses `broadcast-hls` directly instead of routing
  through `transmux` for playlist parsing (issue #878).
