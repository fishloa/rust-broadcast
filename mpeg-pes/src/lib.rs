#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Three runnable examples ship with this crate (`cargo run -p mpeg-pes --example <name>`).\n"]
#![doc = "\n## `parse_pes_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_pes_packet.rs")]
#![doc = "```\n\n## `extract_pts`\n\n```rust,ignore"]
#![doc = include_str!("../examples/extract_pts.rs")]
#![doc = "```\n\n## `build_pes_header`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_pes_header.rs")]
#![doc = "```"]

extern crate alloc;

mod assembler;
mod error;
mod packet;
mod stream_id;
mod timestamp;

pub use assembler::PesAssembler;
pub use error::{Error, Result};
pub use packet::{
    AdditionalCopyInfo, Escr, FREEZE_FRAME_RESERVED_VALUE, PStdBuffer, PesExtension,
    PesExtensionField, PesHeader, PesPacket, ProgramPacketSequenceCounter, TrickMode,
};
pub use stream_id::{STREAM_ID_EXTENSION_AVS3_AUDIO, STREAM_ID_EXTENSION_AVS3_VIDEO, StreamId};
pub use timestamp::{Dts, Pts};

/// The 3-byte `packet_start_code_prefix` that opens every PES packet (`0x000001`).
pub const PACKET_START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];
