#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st337 --example <name>`.\n"]
#![doc = "\n### `build_burst`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_burst.rs")]
#![doc = "```\n\n### `parse_burst`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_burst.rs")]
#![doc = "```"]

extern crate alloc;

mod burst;
mod error;

pub use burst::{
    Burst, BurstPreamble, DataMode, EXTENDED_DATA_TYPE_MARKER, ExtendedPreamble, MAX_5_BIT_FIELD,
    MAX_DATA_STREAM_NUMBER, MAX_LENGTH_CODE_BITS, SYNC_WORD_PA, SYNC_WORD_PB,
};
pub use error::{Error, Result};
