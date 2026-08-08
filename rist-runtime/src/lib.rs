//! RIST Simple Profile (VSF TR-06-1:2020) RTCP message types.
//!
//! Wire-level codecs for the RTCP messages defined (or profiled) by the RIST
//! Simple Profile specification, built on top of the generic
//! [`rtcp_packet`] crate (RFC 3550 §6 SR/RR/SDES/BYE/APP):
//!
//! - [`GenericNack`] — RFC 4585 §6.2.1, RTCP Transport-Layer Feedback
//!   (PT 205, FMT 1). Bitmask-based retransmission request.
//! - [`RangeNack`] — RIST-specific RTCP APP (PT 204, subtype 0,
//!   name `"RIST"`). Range-based retransmission request (TR-06-1 §5.3.2.2).
//! - [`RttEcho`] — RTCP APP (PT 204, name `"RIST"`, subtype 2/3).
//!   Round-trip time measurement (TR-06-1 §5.2.6).
//! - [`RistSenderCompound`] / [`RistReceiverCompound`] — compound RTCP
//!   packet builders enforcing the RIST §5.2.1 structure.
//!
//! All wire types implement the workspace-standard
//! [`Parse`](broadcast_common::Parse)/[`Serialize`](broadcast_common::Serialize)
//! trait pair with byte-exact round-trip fidelity.
//!
//! Depends on `broadcast-common` and `rtcp-packet`. `#![no_std]` (+ `alloc`)
//! when the `std` feature is disabled.
//!
//! # Examples
//!
//! Build a Generic NACK reporting two lost packets and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use rist_runtime::{GenericNack, NackFci};
//!
//! let nack = GenericNack {
//!     ssrc_sender: 0x1111_2222,
//!     ssrc_media: 0x3333_4444,
//!     nacks: vec![
//!         NackFci { pid: 100, blp: 0x0000 },
//!         NackFci { pid: 200, blp: 0x0003 },
//!     ],
//! };
//! let bytes = nack.to_bytes();
//! assert_eq!(GenericNack::parse(&bytes).unwrap(), nack);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod compound;
pub mod error;
pub mod nack;
pub mod rtt_echo;

// ---------------------------------------------------------------------------
// Named constants — VSF TR-06-1:2020 + RFC 4585
// ---------------------------------------------------------------------------

/// RIST APP name — ASCII `"RIST"` (TR-06-1 §5.2.6, §5.3.2.2).
pub const RIST_APP_NAME: [u8; 4] = [0x52, 0x49, 0x53, 0x54];

/// RIST APP name as a `u32` for fast comparison (`0x52495354`).
pub const RIST_APP_NAME_U32: u32 = 0x5249_5354;

/// RTCP PT for Transport-Layer Feedback (RFC 4585 §6.1).
pub const PT_RTPFB: u8 = 205;

/// Generic NACK FMT (RFC 4585 §6.2.1).
pub const FMT_GENERIC_NACK: u8 = 1;

/// RTT Echo Request subtype (TR-06-1 §5.2.6).
pub const SUBTYPE_RTT_ECHO_REQUEST: u8 = 2;

/// RTT Echo Response subtype (TR-06-1 §5.2.6).
pub const SUBTYPE_RTT_ECHO_RESPONSE: u8 = 3;

/// Range NACK subtype (TR-06-1 §5.3.2.2).
pub const SUBTYPE_RANGE_NACK: u8 = 0;

/// Mask for the 5-bit count/FMT/subtype field in RTCP byte 0 (RFC 3550 §6.4.1).
pub(crate) const RTCP_COUNT_MASK: u8 = 0x1F;

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use compound::{RistReceiverCompound, RistSenderCompound};
pub use error::{Error, Result};
pub use nack::{GenericNack, NackFci, PacketRange, RangeNack};
pub use rtt_echo::{RttEcho, RttEchoKind};
