# broadcast-hls 0.1.0

**Release date:** 2026-08-02

Initial release — HLS (M3U8) playlist syntax extracted from `transmux/src/hls.rs` (issue #878) so a consumer that only needs playlist parsing/serialization (`media-doctor`, an HLS-pull client, a fuzz target) doesn't pull in the entire container-muxing hub. `transmux`'s HLS/LL-HLS segmenters (the code that produces container bytes) stay in `transmux` and depend on this crate.

## What's new

- `MediaPlaylist` / `MasterPlaylist` parse + serialize — all 32 RFC 8216bis tags.
- Low-Latency HLS: `LowLatencyConfig`, `PartSpec`, `OpenSegment`, `MapTag`, `RenditionReport`, `SkipInfo`.
- I-frame trick-play: `IFrameVariant`.
- `#EXT-X-VERSION` derivation from the highest-versioned tag present.
- Sub-millisecond `#EXTINF` duration fix (was truncating to three decimal places).
- `#EXT-X-SESSION-KEY` cross-variant validation (RFC 8216bis §4.4.4.4).
- Discontinuity signalling: `mark_init_discontinuities`.
- CENC/CBCS `#EXT-X-KEY` builder: `cenc_ext_x_key` (names `broadcast_common::cenc::CencScheme`).
- `no_std` + `alloc`, depends only on `broadcast-common`.

## Migration

New crate — no migration needed. Consumers currently importing HLS types from `transmux::hls` should migrate to `broadcast_hls::*` (the `transmux` re-exports will be removed in a future major).
