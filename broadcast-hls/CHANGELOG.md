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
- This is a pure move plus one adaptation forced by the dependency direction
  (`transmux` now depends on this crate, not the reverse): a crate-local
  `Error` type, replacing `transmux::Error::HlsParse`. No parsing or rendering
  behaviour changed.
- `CencScheme` (which `cenc_ext_x_key` takes) is **re-exported from
  `broadcast-common` 9.2**, not redefined here — it is the very same type
  `transmux` uses, so nothing converts at the boundary. CENC is *Common*
  Encryption, a container-independent scheme identity, so it lives below both
  crates rather than once per crate (issues #564, #878). `hex_encode` comes
  from `broadcast_common::hex` for the same reason.
- Requires `broadcast-common` **9.2** for those two items.
