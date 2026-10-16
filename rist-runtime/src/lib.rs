#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod arq;
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

/// Mask for the padding (P) bit in RTCP byte 0 (RFC 3550 §6.4.1).
pub(crate) const RTCP_PADDING_MASK: u8 = 0x20;

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use compound::{ReportPart, RistReceiverCompound, RistSenderCompound, UnknownPacket};
pub use error::{Error, Result};
pub use nack::{GenericNack, NackFci, PacketRange, RangeNack};
pub use rtt_echo::{RttEcho, RttEchoKind};
