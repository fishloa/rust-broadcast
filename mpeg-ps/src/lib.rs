//! MPEG-1/2 Program Stream parsing — ISO/IEC 13818-1 (Rec. ITU-T H.222.0) §2.5.
//!
//! The Program Stream (`.mpg` / `.vob`) framing that wraps PES packets: the
//! [`PackHeader`] (42-bit SCR + `program_mux_rate`), the optional
//! [`SystemHeader`] (rate/audio/video bounds + per-stream P-STD buffer bounds),
//! and the [`ProgramStreamMap`] (PSM).
//!
//! PES payloads are parsed via the `mpeg-pes` crate.
//!
//! Depends only on `dvb-common` + `mpeg-pes` and is `#![no_std]` (+ `alloc`).
//!
//! # Examples
//!
//! Parse a pack header from bytes:
//!
//! ```
//! use mpeg_ps::PackHeader;
//! use dvb_common::Parse;
//!
//! // A minimal pack header: start_code 0x000001BA, SCR=0, mux_rate=3, stuffing=0
//! let bytes = [
//!     0x00, 0x00, 0x01, 0xBA,
//!     0x44, 0x00, 0x04, 0x00, 0x04, 0x01,
//!     0x40, 0x00, 0x03, 0x00,
//! ];
//! let h = PackHeader::parse(&bytes).unwrap();
//! assert_eq!(h.program_mux_rate, 3);
//! assert_eq!(h.scr.ticks(), 0);
//! ```
//!
//! Walk a Program Stream:
//!
//! ```no_run
//! # use std::fs;
//! # use mpeg_ps::program_stream;
//! let data = fs::read("tests/fixtures/ffmpeg-mpeg2-ps.mpg").unwrap();
//! let (packs, _trailing) = program_stream::parse_all_packs(&data).unwrap();
//! println!("Found {} packs", packs.len());
//! for (i, pack) in packs.iter().enumerate() {
//!     println!("Pack {}: SCR={} ticks, mux_rate={} B/s",
//!              i, pack.pack_header.scr.ticks(),
//!              pack.pack_header.program_mux_rate * 50);
//! }
//! ```
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

/// The 3-byte `packet_start_code_prefix` that opens PES and PSM packets (`0x000001`).
pub const PACKET_START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// `MPEG_program_end_code` — `0x000001B9`, terminates the program stream.
pub const PROGRAM_END_CODE: u32 = 0x0000_01B9;
