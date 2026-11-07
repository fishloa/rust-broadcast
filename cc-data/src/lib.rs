#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Runnable examples ship with this crate (`cargo run -p cc-data --example <name>`).\n"]
#![doc = "\n## `parse_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_cc_data.rs")]
#![doc = "```\n\n## `build_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_cc_data.rs")]
#![cfg_attr(
    feature = "decode",
    doc = "```\n\n## `decode_cea608`\n\n```rust,ignore"
)]
#![cfg_attr(feature = "decode", doc = include_str!("../examples/decode_cea608.rs"))]
#![cfg_attr(
    feature = "decode",
    doc = "```\n\n## `decode_cea708`\n\n```rust,ignore"
)]
#![cfg_attr(feature = "decode", doc = include_str!("../examples/decode_cea708.rs"))]
#![doc = "```"]

extern crate alloc;

mod cc_data;
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub mod decode;
mod error;

pub use cc_data::{CcData, CcTriplet, CcType};
pub use error::{Error, Result};
