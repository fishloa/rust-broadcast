#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rtcp-packet --example <name>`.\n"]
#![doc = "\n### `build_sender_report`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_sender_report.rs")]
#![doc = "```\n\n### `parse_compound_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_compound_packet.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod packet;

pub use error::{Error, Result};
pub use packet::{
    APP_NAME_LEN, App, Bye, CommonHeader, CompoundPacket, PT_APP, PT_BYE, PT_RECEIVER_REPORT,
    PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION, REPORT_BLOCK_LEN, ReceiverReport, ReportBlock,
    RtcpPacket, RtcpPacketType, SDES_CNAME, SDES_EMAIL, SDES_LOC, SDES_NAME, SDES_NOTE, SDES_PHONE,
    SDES_PRIV, SDES_TOOL, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
};
