#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rtp-packet --example <name>`.\n"]
#![doc = "\n### `build_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_packet.rs")]
#![doc = "```\n\n### `parse_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_packet.rs")]
#![doc = "```\n\n### `rfc8285_extensions` (requires `--features rfc8285`)\n\n```rust,ignore"]
#![doc = include_str!("../examples/rfc8285_extensions.rs")]
#![doc = "```"]

extern crate alloc;

#[cfg(feature = "av3a")]
pub mod av3a;
mod error;
mod header;
#[cfg(feature = "rfc8285")]
pub mod rfc8285;

pub use error::{Error, Result};
pub use header::{
    FIXED_HEADER_LEN, HeaderExtension, MAX_CSRC_COUNT, MAX_PADDING_COUNT, MAX_PAYLOAD_TYPE,
    RTP_VERSION, RtpPacket,
};
