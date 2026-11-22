#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-bbframe --example <name>`).\n"]
#![doc = "\n## `parse_bbheader`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_bbheader.rs")]
#![doc = "```\n\n## `walk_capture`\n\n```rust,ignore"]
#![doc = include_str!("../examples/walk_capture.rs")]
#![doc = "```"]

extern crate alloc;

pub mod crc;
pub mod error;
pub mod header;
pub mod issy;
pub mod packet;
pub mod pump;

pub use error::{Error, Result};
