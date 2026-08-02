# Changelog

All notable changes to `broadcast-hls` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - Unreleased

### Added
- Initial release. HLS (M3U8) playlist syntax (RFC 8216 / RFC 8216bis)
  extracted from `transmux/src/hls.rs` (issue #878): `MediaPlaylist`,
  `MasterPlaylist`, `MediaSegment`, `Variant`, `IFrameVariant`,
  `LowLatencyConfig`, `OpenSegment`, `PartSpec`, `MapTag`, `ByteRange`,
  `PreloadHintType`, `RenditionReport`, `SkipInfo`, `mark_init_discontinuities`,
  `cenc_ext_x_key`. `#![no_std]` + `alloc`; depends only on `broadcast-common`;
  builds for `thumbv7em-none-eabi`.
- This is a pure move plus two adaptations forced by the dependency direction
  (`transmux` now depends on this crate, not the reverse): a local `Error`
  type (previously `transmux::Error::HlsParse`) and a local `CencScheme`
  (previously `transmux::cenc::CencScheme`) used only by `cenc_ext_x_key`'s
  signature. No parsing/rendering behaviour changed.
