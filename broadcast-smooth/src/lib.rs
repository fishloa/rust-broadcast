#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p broadcast-smooth --example <name>`.\n"]
#![doc = "\n### `parse_manifest`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_manifest.rs")]
#![doc = "```\n\n### `build_manifest`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_manifest.rs")]
#![doc = "```"]

extern crate alloc;

mod manifest;

pub use manifest::{
    C, DEFAULT_MAJOR_VERSION, DEFAULT_MINOR_VERSION, MAX_CHUNK_RUN, MAX_CODEC_PRIVATE_DATA_HEX_LEN,
    Protection, QualityLevel, SMOOTH_TIMESCALE, SmoothManifest, SmoothParseError, StreamIndex,
    StreamType, TOKEN_BITRATE, TOKEN_START_TIME, default_limits, hex_decode,
};
