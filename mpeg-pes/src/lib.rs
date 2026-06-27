//! PES (Packetized Elementary Stream) depacketization + PTS/DTS — ISO/IEC 13818-1
//! (Rec. ITU-T H.222.0) §2.4.3.6 / §2.4.3.7.
//!
//! `mpeg-pes` is the sublayer between an MPEG-TS packet layer (e.g. `dvb-si`'s
//! `TsPacket` / `SiDemux`) and an elementary-stream consumer. Feed it the
//! payload bytes of TS packets for one PID (split on `payload_unit_start`), and
//! it yields [`PesPacket`]s carrying the `stream_id`, presentation/decoding
//! timestamps ([`Pts`]/[`Dts`], 33-bit @ 90 kHz), and the elementary-stream
//! bytes.
//!
//! It depends only on `dvb-common` and is `#![no_std]` (+ `alloc`), WASM-clean.
//!
//! ```
//! use mpeg_pes::{PesPacket, StreamId};
//! // A minimal PES packet: start code, stream_id 0xE0 (video), len, header, PTS, payload.
//! let bytes = [
//!     0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05,
//!     0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0
//!     0xAA, 0xBB, // ES payload
//! ];
//! let pkt = PesPacket::parse(&bytes).unwrap();
//! assert_eq!(pkt.stream_id, StreamId(0xE0));
//! assert!(pkt.header.as_ref().unwrap().pts.is_some());
//! assert_eq!(pkt.payload, &[0xAA, 0xBB]);
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p mpeg-pes --example <name>`).\n"]
#![doc = "\n## `parse_pes_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_pes_packet.rs")]
#![doc = "```\n\n## `extract_pts`\n\n```rust,ignore"]
#![doc = include_str!("../examples/extract_pts.rs")]
#![doc = "```"]

extern crate alloc;

mod assembler;
mod error;
mod packet;
mod stream_id;
mod timestamp;

pub use assembler::PesAssembler;
pub use error::{Error, Result};
pub use packet::{PesHeader, PesPacket};
pub use stream_id::StreamId;
pub use timestamp::{Dts, Pts};

/// The 3-byte `packet_start_code_prefix` that opens every PES packet (`0x000001`).
pub const PACKET_START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];
