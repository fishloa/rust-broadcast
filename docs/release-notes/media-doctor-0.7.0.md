# media-doctor 0.7.0

**Release date:** 2026-08-02

Adds an Apple `mediastreamvalidator` oracle test harness (#870) — a genuinely independent HLS conformance check that caught a real gap in `transmux`'s CMAF-HLS packager (missing `#EXT-X-MAP`). Also tightens `hls-part-duration-range` to Error severity and migrates HLS playlist parsing to the new `broadcast-hls` crate.

## What's new

- `mediastreamvalidator_oracle.rs` test harness (issue #870): validates every HLS playlist shape against Apple's own tool. macOS-only; skips loudly on Linux/CI.

## What changed

- `hls-part-duration-range` severity raised from Warning to Error (RFC 8216bis §4.4.4.9 MUST).
- `check_hls_playlist` now uses `broadcast-hls` directly instead of reaching through `transmux` for playlist parsing (issue #878).

## Migration

Requires `broadcast-hls` 0.1. No breaking API changes to this crate's public surface.
