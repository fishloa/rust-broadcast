#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st12-1 --example <name>`.\n"]
#![doc = "\n### `build_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_frame.rs")]
#![doc = "```\n\n### `parse_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_frame.rs")]
#![doc = "```"]

mod error;
mod frame;

pub use error::{Error, Result};
pub use frame::{
    BinaryGroupFlags, BinaryGroupUsage, FRAME_LEN, FrameRate, LtcFrame, MAX_BINARY_GROUP,
    MAX_FRAMES, MAX_HOURS, MAX_MINUTES_SECONDS, SYNC_WORD,
};
