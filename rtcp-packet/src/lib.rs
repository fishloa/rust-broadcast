//! RTCP control packets — RFC 3550 §6, spec-complete (not just a happy-path
//! subset).
//!
//! Typed, symmetric [`Parse`](broadcast_common::Parse)/[`Serialize`](broadcast_common::Serialize)
//! for every RTCP packet type described in the curated spec transcription at
//! `rtcp-packet/docs/rtcp.md` (fetched directly from
//! [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550.txt)) — cite that file,
//! not this doc comment, as the field-semantics oracle.
//!
//! - [`SenderReport`] — SR (§6.4.1, PT 200).
//! - [`ReceiverReport`] — RR (§6.4.2, PT 201).
//! - [`ReportBlock`] — the 24-byte reception report block shared by SR/RR.
//! - [`SourceDescription`] / [`SdesChunk`] / [`SdesItem`] / [`SdesItemType`]
//!   — SDES (§6.5, PT 202).
//! - [`Bye`] — BYE (§6.6, PT 203).
//! - [`App`] — APP (§6.7, PT 204).
//! - [`RtcpPacket`] / [`RtcpPacketType`] — the packet-type dispatch enum.
//! - [`CompoundPacket`] — §6.1's compound packet (a sequence of RTCP packets
//!   that must begin with SR or RR), with byte-exact round-trip.
//!
//! Two decode-completeness gaps are documented (not silently glossed over)
//! in `docs/rtcp.md`: SR/RR profile-specific extensions and the SDES PRIV
//! item's internal `prefix`/`value` sub-structure are not separately typed.
//!
//! RTCP carries **no media** — this is a standalone wire codec for the RTP
//! control channel (a companion to the `rtp-packet` crate), not a hub
//! `Package`/`Unpackage` spoke.
//!
//! Depends only on `broadcast-common`. `#![no_std]` (+ `alloc`) when the
//! `std` feature is disabled.
//!
//! # Examples
//!
//! Build a Sender Report with two reception report blocks and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use rtcp_packet::{ReportBlock, SenderReport};
//!
//! let sr = SenderReport {
//!     ssrc: 0x1122_3344,
//!     ntp_msw: 0xE0E1_E2E3,
//!     ntp_lsw: 0x1020_3040,
//!     rtp_timestamp: 0x0009_0000,
//!     packet_count: 4321,
//!     octet_count: 999_999,
//!     report_blocks: vec![ReportBlock {
//!         ssrc: 0xAAAA_AAAA,
//!         fraction_lost: 12,
//!         cumulative_lost: -3,
//!         ext_highest_seq: 0x0001_2345,
//!         jitter: 500,
//!         lsr: 0xAABB_CCDD,
//!         dlsr: 0x0000_1000,
//!     }],
//! };
//! let mut bytes = vec![0u8; sr.serialized_len()];
//! sr.serialize_into(&mut bytes).unwrap();
//! assert_eq!(SenderReport::parse(&bytes).unwrap(), sr);
//! ```
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
