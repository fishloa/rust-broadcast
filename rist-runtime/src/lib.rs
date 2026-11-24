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

/// Common-header length in bytes (RFC 3550 §6.1).
pub(crate) const RTCP_HEADER_LEN: usize = 4;

/// One 32-bit word, in bytes — the unit of the common-header `length` field
/// (RFC 3550 §6.1).
pub(crate) const WORD_LEN: usize = 4;

/// Byte offset of the common header's packet-type field (RFC 3550 §6.1).
pub(crate) const RTCP_LENGTH_OFFSET: usize = 2;

/// RTCP packet type for an application-defined packet (RFC 3550 §6.7).
pub(crate) const PT_APP: u8 = 204;

/// RTCP protocol version — always 2 (RFC 3550 §6.4.1).
pub(crate) const RTCP_VERSION: u8 = 2;

/// Shift of the 2-bit `version` field in the common header's first byte
/// (RFC 3550 §6.1).
pub(crate) const VERSION_SHIFT: u32 = 6;

/// Read the big-endian 32-bit field at `offset` of `bytes`.
pub(crate) fn be_u32(bytes: &[u8], offset: usize) -> u32 {
    const FIELD_LEN: usize = core::mem::size_of::<u32>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u32::from_be_bytes(field)
}

/// Read the big-endian 16-bit field at `offset` of `bytes`.
pub(crate) fn be_u16(bytes: &[u8], offset: usize) -> u16 {
    const FIELD_LEN: usize = core::mem::size_of::<u16>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u16::from_be_bytes(field)
}

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use compound::{ReportPart, RistReceiverCompound, RistSenderCompound, UnknownPacket};
pub use error::{Error, Result};
pub use nack::{GenericNack, NackFci, PacketRange, RangeNack};
pub use rtt_echo::{RttEcho, RttEchoKind};
