# mpeg-ps

[![Crates.io](https://img.shields.io/crates/v/mpeg-ps.svg)](https://crates.io/crates/mpeg-ps)
[![docs.rs](https://img.shields.io/docsrs/mpeg-ps)](https://docs.rs/mpeg-ps)

MPEG-1/2 Program Stream parser — ISO/IEC 13818-1 (Rec. ITU-T H.222.0) §2.5.

Parses the `.mpg`/`.vob` framing that wraps PES packets: the pack header
(42-bit SCR + `program_mux_rate`), the optional system header (rate/audio/video
bounds + per-stream P-STD buffer bounds), and the program stream map (PSM).

`#![no_std]` + `alloc`; depends only on `broadcast-common` and `mpeg-pes`.

## Quick start

```rust
use mpeg_ps::PackHeader;
use broadcast_common::Parse;

let bytes = [
    0x00, 0x00, 0x01, 0xBA,
    0x44, 0x00, 0x04, 0x00, 0x04, 0x01,
    0x40, 0x00, 0x03, 0x00,
];
let h = PackHeader::parse(&bytes).unwrap();
assert_eq!(h.program_mux_rate, 3);
```

```rust
use std::fs;
use mpeg_ps::program_stream;

let data = fs::read("tests/fixtures/ffmpeg-mpeg2-ps.mpg").unwrap();
let (packs, _) = program_stream::parse_all_packs(&data).unwrap();
println!("Found {} packs", packs.len());
```

## Examples

```sh
cargo run -p mpeg-ps --example parse_pack_header
cargo run -p mpeg-ps --example walk_ps
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
