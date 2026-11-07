#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st291 --example <name>`.\n"]
#![doc = "\n### `build_anc`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_anc.rs")]
#![doc = "```\n\n### `parse_anc`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_anc.rs")]
#![doc = "```\n\n### `build_anc_rtp`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_anc_rtp.rs")]
#![doc = "```\n\n### `parse_anc_rtp`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_anc_rtp.rs")]
#![doc = "```"]

extern crate alloc;

#[cfg(feature = "ts")]
mod anc;
mod anc_content;
#[cfg(feature = "ts")]
mod descriptor;
mod error;
#[cfg(feature = "rtp")]
mod rtp;

#[cfg(feature = "ts")]
pub use anc::{
    ANC_PES_HEADER_DATA_LENGTH, ANC_STREAM_ID, AncDataPacket, AncPacket, PACKET_START_CODE_PREFIX,
    STUFFING_BYTE,
};
pub use anc_content::{AncContent, ValidationReport};
#[cfg(feature = "ts")]
pub use descriptor::{
    ANC_DATA_DESCRIPTOR_TAG, ANC_STREAM_TYPE, AncDataDescriptor, VANC_FORMAT_IDENTIFIER,
    VANC_FORMAT_IDENTIFIER_BYTES,
};
pub use error::{Error, Result};
#[cfg(feature = "rtp")]
pub use rtp::{
    ANC_RTP_DEFAULT_CLOCK_RATE, ANC_RTP_MEDIA_TYPE, ANC_RTP_PAYLOAD_HEADER_LEN, AncRtpPayload,
    FieldSense, RtpAncPacket,
};
