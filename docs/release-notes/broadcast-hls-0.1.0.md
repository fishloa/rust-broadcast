# broadcast-hls 0.1.0

Released 2026-08-02.

### Added

Initial release — HLS (M3U8) playlist syntax (RFC 8216 / RFC 8216bis):
`MediaPlaylist`/`MasterPlaylist` parse + serialize, Low-Latency HLS
(`LowLatencyConfig`/`PartSpec`/`OpenSegment`/`MapTag`/`RenditionReport`/
`SkipInfo`), I-frame trick-play (`IFrameVariant`), discontinuity signalling
(`mark_init_discontinuities`), and CENC/CBCS `#EXT-X-KEY` signalling
(`cenc_ext_x_key`). Extracted from `transmux/src/hls.rs` (issue #878).
`no_std` + `alloc`, depends only on `broadcast-common`.
