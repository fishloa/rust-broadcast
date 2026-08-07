# transmux 0.22.0

**Release date:** 2026-08-02

HLS playlist syntax (`MediaPlaylist`, `MasterPlaylist`, and all related types) moved out to the new `broadcast-hls` crate (issue #878). `CencScheme` and `hex_encode` moved down to `broadcast-common` 9.2 so sibling crates can share them without depending on `transmux`. The CMAF-HLS `HlsPackager` now emits `#EXT-X-MAP` on every Media Segment, fixing a real conformance gap caught by Apple's `mediastreamvalidator` oracle (#870).

## What's changed

- **HLS playlist syntax extracted to `broadcast-hls`** — `transmux::hls` and every `MediaPlaylist`/`MasterPlaylist`/`MediaSegment`/`Variant`/etc. path no longer exists. No compatibility re-export. `Error::HlsParse` removed from `transmux::Error`.
- **`CencScheme` moved to `broadcast-common::cenc`** — `transmux::CencScheme` is now a re-export. `CencScheme` is `#[non_exhaustive]` across a crate boundary; new `Error::UnsupportedCencScheme` for future scheme additions.
- **`hex_encode` moved to `broadcast-common::hex`** — `transmux::rtp::hex_encode` is now a re-export.
- **`HlsPackager` conformance fix** — now emits `#EXT-X-MAP` with a `BYTERANGE` covering the `ftyp`+`moov` span on every CMAF segment (RFC 8216bis requirement).

## Migration

Breaking. Depend on `broadcast-hls` directly for HLS playlist types; `transmux` no longer re-exports them. Requires `broadcast-common` 9.2.
