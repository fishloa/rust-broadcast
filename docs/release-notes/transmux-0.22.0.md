# transmux 0.22.0

**Release date:** 2026-08-02

Extracts HLS playlist syntax into the new `broadcast-hls` crate (issue #878) and moves `CencScheme` down to `broadcast-common` 9.2.0 so both `transmux` and `broadcast-hls` can name it without a circular dependency. The segmenters (`ts_hls`, `ll_hls` — the code that produces container bytes) stay in `transmux` and depend on `broadcast-hls` for playlist rendering.

## What changed

- **HLS playlist syntax extracted** to `broadcast-hls` 0.1 (issue #878). `transmux::hls::MediaPlaylist`, `MasterPlaylist`, and all M3U8 tag types are re-exported from `broadcast-hls` — existing callers compile but should migrate imports.
- **`CencScheme` moved** to `broadcast-common::cenc` (was `transmux::cenc`). Re-exported from the old path for backwards compat.
- Requires `broadcast-common` 9.2, `broadcast-hls` 0.1.

## Migration

Replace `transmux::hls::{MediaPlaylist, MasterPlaylist, ...}` imports with `broadcast_hls::*`. Replace `transmux::cenc::CencScheme` with `broadcast_common::cenc::CencScheme`. Both old paths still compile via re-exports but will be removed in a future major.
