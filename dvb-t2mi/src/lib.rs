#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-t2mi --example <name>`).\n"]
#![doc = "\n## `parse_header`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_header.rs")]
#![doc = "```\n\n## `pump_capture`\n\n```rust,ignore"]
#![doc = include_str!("../examples/pump_capture.rs")]
#![doc = "```"]

extern crate alloc;

pub mod crc;
pub mod error;
pub mod packet;
pub mod payload;
pub mod traits;

#[cfg(feature = "ts")]
pub mod pump;

#[cfg(feature = "ts")]
pub mod ts;

#[cfg(feature = "ts")]
pub mod inner_ts;

pub use error::{Error, Result};
