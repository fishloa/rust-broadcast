#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p mpeg-ps --example <name>`.\n"]
#![doc = "\n### `parse_pack_header`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_pack_header.rs")]
#![doc = "```\n\n### `walk_ps`\n\n```rust,ignore"]
#![doc = include_str!("../examples/walk_ps.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod pack_header;
pub mod program_stream;
pub mod program_stream_map;
mod scr;
mod system_header;

pub use error::{Error, Result};
pub use pack_header::PackHeader;
pub use program_stream_map::{EsMapEntry, ProgramStreamMap};
pub use scr::Scr;
pub use system_header::{StdBufferBound, SystemHeader};

/// The 3-byte `packet_start_code_prefix` that opens PES and PSM packets
/// (`0x000001`) — the one definition lives in `mpeg-pes` (#1141).
pub use mpeg_pes::PACKET_START_CODE_PREFIX;

/// `MPEG_program_end_code` — `0x000001B9`, terminates the program stream.
pub const PROGRAM_END_CODE: u32 = 0x0000_01B9;
