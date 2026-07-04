//! SRT packet structure — `draft-sharabayko-srt-01` §3 (Packet Structure).
//!
//! Every SRT packet is the payload of one UDP datagram (§3, Figure 1). It
//! starts with a 16-byte SRT header (Figure 2): a leading `F` bit
//! distinguishes data packets (`F=0`, §3.1) from control packets (`F=1`,
//! §3.2); the following two header words carry packet-type-specific fields,
//! and the header always closes with a 32-bit Timestamp and a 32-bit
//! Destination Socket ID.
//!
//! This module is packet **structure** only — parse/serialize of the wire
//! format. The handshake *state machine* (caller/listener/rendezvous
//! exchange, §4.3), loss/ARQ handling, TSBPD, congestion control, and the
//! actual AES key-wrap/unwrap crypto (§6) are explicit follow-ups; see the
//! crate root docs.

pub mod ack;
pub mod control;
pub mod data;
pub mod handshake;
pub mod key_material;
pub mod misc;
pub mod nak;

pub use ack::{AckCif, AckPacket};
pub use control::{ControlPacket, ControlType, UserDefinedPacket};
pub use data::{DataPacket, EncryptionKeyField, PacketPosition};
pub use handshake::{
    EncryptionField, ExtensionType, GroupFlags, GroupMembershipExtension, GroupType,
    HandshakeExtensionBlock, HandshakeExtensionFlags, HandshakeExtensionMessageFlags,
    HandshakeExtensions, HandshakePacket, HandshakeType, HsExtMessage,
};
pub use key_material::{Cipher, KeyMaterial, KmAuth, KmKeyFlag, StreamEncapsulation};
pub use misc::{
    AckAckPacket, CongestionWarningPacket, DropReqPacket, KeepAlivePacket, PeerErrorPacket,
    ShutdownPacket,
};
pub use nak::{LossListEntry, NakPacket};

use crate::error::{Error, Result};

/// Length in bytes of the fixed SRT header shared by data and control packets
/// (`draft-sharabayko-srt-01` §3, Figure 2): 4 header words = 16 bytes, before
/// any packet-type-specific payload (data) or Control Information Field
/// (control).
pub const SRT_HEADER_LEN: usize = 16;

/// The `F` (Packet Type Flag) bit — bit 31 of the first header word. Clear for
/// a data packet, set for a control packet (§3, Figure 2). Also reused,
/// bit-for-bit, as the range marker in the NAK loss-list coding (Appendix A).
pub(crate) const F_BIT: u32 = 0x8000_0000;

/// Mask for a 31-bit packet sequence number (data packet §3.1, NAK loss list
/// Appendix A): all bits below the `F`/range-marker bit.
pub(crate) const SEQ_NUMBER_MASK: u32 = 0x7FFF_FFFF;

/// A parsed SRT packet — the payload of one UDP datagram carrying SRT traffic
/// (`draft-sharabayko-srt-01` §3, Figure 1 / Figure 2).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SrtPacket<'a> {
    /// `F=0`: a data packet (§3.1).
    Data(DataPacket<'a>),
    /// `F=1`: a control packet (§3.2).
    Control(ControlPacket<'a>),
}

impl<'a> SrtPacket<'a> {
    /// Parse one SRT packet from `bytes` — the full payload of one UDP
    /// datagram carrying SRT traffic. Dispatches on the `F` bit of the first
    /// header word (§3, Figure 2).
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < SRT_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: SRT_HEADER_LEN,
                have: bytes.len(),
                what: "SRT header",
            });
        }
        let word0 = be32(bytes, 0);
        if word0 & F_BIT != 0 {
            Ok(SrtPacket::Control(ControlPacket::parse(bytes)?))
        } else {
            Ok(SrtPacket::Data(DataPacket::parse(bytes)?))
        }
    }

    /// Number of bytes [`Self::serialize_into`] will write.
    pub fn serialized_len(&self) -> usize {
        match self {
            SrtPacket::Data(d) => d.serialized_len(),
            SrtPacket::Control(c) => c.serialized_len(),
        }
    }

    /// Serialize this packet into `buf`. Returns the number of bytes written.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            SrtPacket::Data(d) => d.serialize_into(buf),
            SrtPacket::Control(c) => c.serialize_into(buf),
        }
    }
}

/// Read a big-endian `u32` at byte offset `off`.
///
/// # Panics
/// Panics if `bytes.len() < off + 4`. Every call site slices/length-checks
/// first, so this is an internal invariant, not an external input path.
pub(crate) fn be32(bytes: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

/// Write a big-endian `u32` at byte offset `off`.
///
/// # Panics
/// Panics if `buf.len() < off + 4`; see [`be32`].
pub(crate) fn put_be32(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_be_bytes());
}

/// Read a big-endian `u16` at byte offset `off`. See [`be32`] on panics.
pub(crate) fn be16(bytes: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([bytes[off], bytes[off + 1]])
}

/// Write a big-endian `u16` at byte offset `off`. See [`put_be32`] on panics.
pub(crate) fn put_be16(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_be_bytes());
}
