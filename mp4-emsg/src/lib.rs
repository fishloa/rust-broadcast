#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p mp4-emsg --example <name>`.\n"]
#![doc = "\n### `build_emsg`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_emsg.rs")]
#![doc = "```\n\n### `parse_emsg`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_emsg.rs")]
#![doc = "```"]

extern crate alloc;

mod emsg;
mod error;
mod version;

pub use emsg::{
    EMSG_BOX_TYPE, EMSG_FLAGS, EmsgBox, FULLBOX_HEADER_LEN, PresentationTime, SCTE35_SCHEME_PREFIX,
    STRING_TERMINATOR,
};
pub use error::{Error, Result};
pub use version::{EmsgVersion, VERSION_0, VERSION_1};
