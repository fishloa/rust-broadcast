# broadcast-hls

[![Crates.io](https://img.shields.io/crates/v/broadcast-hls.svg)](https://crates.io/crates/broadcast-hls)
[![docs.rs](https://img.shields.io/docsrs/broadcast-hls)](https://docs.rs/broadcast-hls)

> **Extracted from `transmux`** (issue #878). `transmux/src/hls.rs` — the M3U8
> playlist syntax — moved here so a crate that only needs to parse/render HLS
> playlists (e.g. `media-doctor`, an HLS-pull client, a fuzz target) no longer
> has to pull in an entire any-to-any container muxing hub to parse a text
> file. `transmux`'s own HLS/LL-HLS **segmenters** (`ts_hls`, `ll_hls` — they
> produce container bytes, not playlist syntax) stayed put.

## Install

```toml
[dependencies]
broadcast-hls = "0.1"
```

HLS (M3U8) playlist syntax — RFC 8216 / RFC 8216bis (Low-Latency HLS): typed
Media/Master Playlist parse + serialize.

Implements:

- **`MediaPlaylist`** / **`MasterPlaylist`** — `#EXTM3U` Media and
  Multivariant (Master) Playlists; `to_m3u8()` renders, `parse()` is its
  symmetric inverse.
- **`MediaSegment`** / **`Variant`** / **`IFrameVariant`** — per-segment and
  per-variant entries, including RFC 8216 §4.3.4.2 I-frame-only trick-play
  signalling.
- **Low-Latency HLS (RFC 8216bis)** — `LowLatencyConfig`, `PartSpec`,
  `OpenSegment`, `MapTag`, `ByteRange`, `PreloadHintType`,
  `RenditionReport`, `SkipInfo` — the partial-segment / blocking-reload /
  playlist-delta-update directives, all strictly opt-in so a plain playlist
  is byte-for-byte unchanged.
- **`#EXT-X-DISCONTINUITY[-SEQUENCE]`** signalling, including
  `mark_init_discontinuities` (auto-detect an init-segment change across a
  segment run).
- **CENC/CBCS DRM signalling** (ISO/IEC 23001-7, issue #564) —
  `cenc_ext_x_key` renders the `#EXT-X-KEY` tag for a `cbcs`-protected CMAF
  track (`cenc`/AES-CTR has no valid HLS `METHOD`, so it returns `None`).

`#![no_std]` + `alloc`; depends only on `broadcast-common`. Builds for
`thumbv7em-none-eabi`.

## Quick start

```rust
use broadcast_hls::{MediaPlaylist, MediaSegment};

let playlist = MediaPlaylist {
    version: 3,
    target_duration: 10,
    segments: vec![MediaSegment {
        uri: "seg0.m4s".into(),
        duration: 9.009,
        ..Default::default()
    }],
    endlist: true,
    ..Default::default()
};
let m3u8 = playlist.to_m3u8();
assert_eq!(MediaPlaylist::parse(&m3u8).unwrap(), playlist);
```

## Examples

```sh
cargo run -p broadcast-hls --example build_playlist
cargo run -p broadcast-hls --example parse_playlist
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
