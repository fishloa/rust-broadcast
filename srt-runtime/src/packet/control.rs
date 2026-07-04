//! SRT control packet dispatch — `draft-sharabayko-srt-01` §3.2, Figure 4,
//! Table 1.
//!
//! ```text
//! word0  1|         Control Type (15)   |          Subtype (16)         |
//! word1                   Type-specific Information (32)
//! word2                          Timestamp (32)
//! word3                    Destination Socket ID (32)
//! rest                              CIF (variable, per Control Type)
//! ```
//!
//! `Subtype` is `0x0` for every Control Type in Table 1 except
//! `User-Defined Type` (`0x7FFF`), where it carries `SRT_CMD_KMREQ` /
//! `SRT_CMD_KMRSP` (Table 5) when the CIF is a Key Material message (§3.2.2).
//! `Subtype` and the `Type-specific Information` word (where a given packet
//! type does not use it) are validated as reserved-must-be-zero and are not
//! stored in the typed per-type structs — see the crate root's reserved-bit
//! policy.

use super::ack::AckPacket;
use super::handshake::HandshakePacket;
use super::misc::{
    AckAckPacket, CongestionWarningPacket, DropReqPacket, KeepAlivePacket, PeerErrorPacket,
    ShutdownPacket,
};
use super::nak::NakPacket;
use super::{Error, Result, SRT_HEADER_LEN, be32, put_be32};

/// `Control Type` wire values (`draft-sharabayko-srt-01` §3.2, Table 1).
pub const CONTROL_TYPE_HANDSHAKE: u16 = 0x0000;
/// Keep-Alive (§3.2.3).
pub const CONTROL_TYPE_KEEPALIVE: u16 = 0x0001;
/// ACK (§3.2.4).
pub const CONTROL_TYPE_ACK: u16 = 0x0002;
/// NAK / Loss Report (§3.2.5).
pub const CONTROL_TYPE_NAK: u16 = 0x0003;
/// Congestion Warning (§3.2.6).
pub const CONTROL_TYPE_CONGESTION_WARNING: u16 = 0x0004;
/// Shutdown (§3.2.7).
pub const CONTROL_TYPE_SHUTDOWN: u16 = 0x0005;
/// ACKACK (§3.2.8).
pub const CONTROL_TYPE_ACKACK: u16 = 0x0006;
/// Message Drop Request (§3.2.9).
pub const CONTROL_TYPE_DROPREQ: u16 = 0x0007;
/// Peer Error (§3.2.10).
pub const CONTROL_TYPE_PEERERROR: u16 = 0x0008;
/// User-Defined Type (reserved value; also carries the Key Material message
/// per §3.2.2 via `Subtype` `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP`).
pub const CONTROL_TYPE_USER_DEFINED: u16 = 0x7FFF;

/// The `Control Type` field (`draft-sharabayko-srt-01` §3.2, Table 1) — 15
/// bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ControlType {
    /// `0x0000` — Handshake (§3.2.1).
    Handshake,
    /// `0x0001` — Keep-Alive (§3.2.3).
    KeepAlive,
    /// `0x0002` — ACK (§3.2.4).
    Ack,
    /// `0x0003` — NAK / Loss Report (§3.2.5).
    Nak,
    /// `0x0004` — Congestion Warning (§3.2.6).
    CongestionWarning,
    /// `0x0005` — Shutdown (§3.2.7).
    Shutdown,
    /// `0x0006` — ACKACK (§3.2.8).
    AckAck,
    /// `0x0007` — Message Drop Request (§3.2.9).
    DropReq,
    /// `0x0008` — Peer Error (§3.2.10).
    PeerError,
    /// `0x7FFF` — User-Defined Type (also the Key Material control-packet
    /// delivery form, §3.2.2).
    UserDefined,
    /// A Control Type value Table 1 does not define.
    Reserved(u16),
}

impl ControlType {
    /// Decode a 15-bit `Control Type` value.
    pub fn from_bits(v: u16) -> Self {
        match v {
            CONTROL_TYPE_HANDSHAKE => ControlType::Handshake,
            CONTROL_TYPE_KEEPALIVE => ControlType::KeepAlive,
            CONTROL_TYPE_ACK => ControlType::Ack,
            CONTROL_TYPE_NAK => ControlType::Nak,
            CONTROL_TYPE_CONGESTION_WARNING => ControlType::CongestionWarning,
            CONTROL_TYPE_SHUTDOWN => ControlType::Shutdown,
            CONTROL_TYPE_ACKACK => ControlType::AckAck,
            CONTROL_TYPE_DROPREQ => ControlType::DropReq,
            CONTROL_TYPE_PEERERROR => ControlType::PeerError,
            CONTROL_TYPE_USER_DEFINED => ControlType::UserDefined,
            other => ControlType::Reserved(other),
        }
    }

    /// The wire value.
    pub fn to_bits(self) -> u16 {
        match self {
            ControlType::Handshake => CONTROL_TYPE_HANDSHAKE,
            ControlType::KeepAlive => CONTROL_TYPE_KEEPALIVE,
            ControlType::Ack => CONTROL_TYPE_ACK,
            ControlType::Nak => CONTROL_TYPE_NAK,
            ControlType::CongestionWarning => CONTROL_TYPE_CONGESTION_WARNING,
            ControlType::Shutdown => CONTROL_TYPE_SHUTDOWN,
            ControlType::AckAck => CONTROL_TYPE_ACKACK,
            ControlType::DropReq => CONTROL_TYPE_DROPREQ,
            ControlType::PeerError => CONTROL_TYPE_PEERERROR,
            ControlType::UserDefined => CONTROL_TYPE_USER_DEFINED,
            ControlType::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            ControlType::Handshake => "handshake",
            ControlType::KeepAlive => "keep-alive",
            ControlType::Ack => "ACK",
            ControlType::Nak => "NAK",
            ControlType::CongestionWarning => "congestion warning",
            ControlType::Shutdown => "shutdown",
            ControlType::AckAck => "ACKACK",
            ControlType::DropReq => "message drop request",
            ControlType::PeerError => "peer error",
            ControlType::UserDefined => "user-defined",
            ControlType::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(ControlType, Reserved);

/// A Control Type = User-Defined (`0x7FFF`) packet, or a Control Type not
/// defined by Table 1. The CIF is genuinely opaque here — the only defined
/// use is the Key Material message (§3.2.2) via `Subtype`
/// `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP` — see [`Self::as_key_material`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct UserDefinedPacket<'a> {
    /// The raw 15-bit Control Type value.
    pub control_type: u16,
    /// The 16-bit Subtype — `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP` when the CIF is
    /// Key Material (Table 5), otherwise vendor-defined.
    pub subtype: u16,
    /// The header `Type-specific Information` word.
    pub type_specific_info: u32,
    /// Timestamp (§3).
    pub timestamp: u32,
    /// Destination Socket ID (§3).
    pub dest_socket_id: u32,
    /// The raw CIF bytes.
    pub cif: &'a [u8],
}

impl<'a> UserDefinedPacket<'a> {
    /// Attempt to decode [`Self::cif`] as a Key Material message (§3.2.2).
    /// Valid regardless of [`Self::subtype`] — callers that care should check
    /// `subtype` against `SRT_CMD_KMREQ`/`SRT_CMD_KMRSP` (Table 5) first.
    pub fn as_key_material(&self) -> Result<super::KeyMaterial<'a>> {
        super::KeyMaterial::parse(self.cif)
    }
}

/// An SRT control packet (`draft-sharabayko-srt-01` §3.2).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ControlPacket<'a> {
    /// Handshake (§3.2.1).
    Handshake(HandshakePacket<'a>),
    /// Keep-Alive (§3.2.3).
    KeepAlive(KeepAlivePacket),
    /// ACK (§3.2.4).
    Ack(AckPacket),
    /// NAK / Loss Report (§3.2.5).
    Nak(NakPacket<'a>),
    /// Congestion Warning (§3.2.6).
    CongestionWarning(CongestionWarningPacket),
    /// Shutdown (§3.2.7).
    Shutdown(ShutdownPacket),
    /// ACKACK (§3.2.8).
    AckAck(AckAckPacket),
    /// Message Drop Request (§3.2.9).
    DropReq(DropReqPacket),
    /// Peer Error (§3.2.10).
    PeerError(PeerErrorPacket),
    /// User-Defined Type, or a Control Type Table 1 does not define.
    UserDefined(UserDefinedPacket<'a>),
}

fn check_reserved_u16(what: &'static str, v: u16) -> Result<()> {
    if v != 0 {
        return Err(Error::ReservedFieldNotZero {
            what,
            value: u64::from(v),
        });
    }
    Ok(())
}

fn check_reserved_u32(what: &'static str, v: u32) -> Result<()> {
    if v != 0 {
        return Err(Error::ReservedFieldNotZero {
            what,
            value: u64::from(v),
        });
    }
    Ok(())
}

fn check_no_cif(what: &'static str, cif: &[u8]) -> Result<()> {
    if !cif.is_empty() {
        return Err(Error::UnexpectedTrailingBytes {
            what,
            extra: cif.len(),
        });
    }
    Ok(())
}

impl<'a> ControlPacket<'a> {
    /// Parse a control packet from `bytes` (the full SRT packet).
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if shorter than the 16-byte header;
    /// [`Error::WrongPacketKind`] if the `F` bit is clear (this is a data
    /// packet); type-specific errors from the dispatched CIF parser.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < SRT_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: SRT_HEADER_LEN,
                have: bytes.len(),
                what: "SRT control packet header",
            });
        }
        let word0 = be32(bytes, 0);
        if word0 & super::F_BIT == 0 {
            return Err(Error::WrongPacketKind {
                expected: "control packet (F=1)",
            });
        }
        let control_type_bits = ((word0 >> 16) & 0x7FFF) as u16;
        let subtype = (word0 & 0xFFFF) as u16;
        let type_specific_info = be32(bytes, 4);
        let timestamp = be32(bytes, 8);
        let dest_socket_id = be32(bytes, 12);
        let cif = &bytes[SRT_HEADER_LEN..];
        let control_type = ControlType::from_bits(control_type_bits);

        Ok(match control_type {
            ControlType::Handshake => {
                check_reserved_u16("Subtype", subtype)?;
                check_reserved_u32("Type-specific Information", type_specific_info)?;
                ControlPacket::Handshake(HandshakePacket::parse_cif(
                    timestamp,
                    dest_socket_id,
                    cif,
                )?)
            }
            ControlType::KeepAlive => {
                check_reserved_u16("Subtype", subtype)?;
                check_reserved_u32("Type-specific Information", type_specific_info)?;
                check_no_cif("keep-alive CIF", cif)?;
                ControlPacket::KeepAlive(KeepAlivePacket {
                    timestamp,
                    dest_socket_id,
                })
            }
            ControlType::Ack => {
                check_reserved_u16("Subtype", subtype)?;
                ControlPacket::Ack(AckPacket::parse_cif(
                    type_specific_info,
                    timestamp,
                    dest_socket_id,
                    cif,
                )?)
            }
            ControlType::Nak => {
                check_reserved_u16("Subtype", subtype)?;
                check_reserved_u32("Type-specific Information", type_specific_info)?;
                ControlPacket::Nak(NakPacket::parse_cif(timestamp, dest_socket_id, cif))
            }
            ControlType::CongestionWarning => {
                check_reserved_u16("Subtype", subtype)?;
                check_reserved_u32("Type-specific Information", type_specific_info)?;
                check_no_cif("congestion warning CIF", cif)?;
                ControlPacket::CongestionWarning(CongestionWarningPacket {
                    timestamp,
                    dest_socket_id,
                })
            }
            ControlType::Shutdown => {
                check_reserved_u16("Subtype", subtype)?;
                check_reserved_u32("Type-specific Information", type_specific_info)?;
                check_no_cif("shutdown CIF", cif)?;
                ControlPacket::Shutdown(ShutdownPacket {
                    timestamp,
                    dest_socket_id,
                })
            }
            ControlType::AckAck => {
                check_reserved_u16("Subtype", subtype)?;
                check_no_cif("ACKACK CIF", cif)?;
                ControlPacket::AckAck(AckAckPacket {
                    ack_number: type_specific_info,
                    timestamp,
                    dest_socket_id,
                })
            }
            ControlType::DropReq => {
                check_reserved_u16("Subtype", subtype)?;
                ControlPacket::DropReq(DropReqPacket::parse_cif(
                    type_specific_info,
                    timestamp,
                    dest_socket_id,
                    cif,
                )?)
            }
            ControlType::PeerError => {
                check_reserved_u16("Subtype", subtype)?;
                check_no_cif("peer error CIF", cif)?;
                ControlPacket::PeerError(PeerErrorPacket {
                    error_code: type_specific_info,
                    timestamp,
                    dest_socket_id,
                })
            }
            ControlType::UserDefined | ControlType::Reserved(_) => {
                ControlPacket::UserDefined(UserDefinedPacket {
                    control_type: control_type_bits,
                    subtype,
                    type_specific_info,
                    timestamp,
                    dest_socket_id,
                    cif,
                })
            }
        })
    }

    /// The `Control Type` this packet carries.
    pub fn control_type(&self) -> ControlType {
        match self {
            ControlPacket::Handshake(_) => ControlType::Handshake,
            ControlPacket::KeepAlive(_) => ControlType::KeepAlive,
            ControlPacket::Ack(_) => ControlType::Ack,
            ControlPacket::Nak(_) => ControlType::Nak,
            ControlPacket::CongestionWarning(_) => ControlType::CongestionWarning,
            ControlPacket::Shutdown(_) => ControlType::Shutdown,
            ControlPacket::AckAck(_) => ControlType::AckAck,
            ControlPacket::DropReq(_) => ControlType::DropReq,
            ControlPacket::PeerError(_) => ControlType::PeerError,
            ControlPacket::UserDefined(u) => ControlType::from_bits(u.control_type),
        }
    }

    fn subtype(&self) -> u16 {
        match self {
            ControlPacket::UserDefined(u) => u.subtype,
            _ => 0,
        }
    }

    fn word1(&self) -> u32 {
        match self {
            ControlPacket::Handshake(_)
            | ControlPacket::KeepAlive(_)
            | ControlPacket::Nak(_)
            | ControlPacket::CongestionWarning(_)
            | ControlPacket::Shutdown(_) => 0,
            ControlPacket::Ack(a) => a.ack_number,
            ControlPacket::AckAck(a) => a.ack_number,
            ControlPacket::DropReq(d) => d.message_number,
            ControlPacket::PeerError(p) => p.error_code,
            ControlPacket::UserDefined(u) => u.type_specific_info,
        }
    }

    fn timestamp(&self) -> u32 {
        match self {
            ControlPacket::Handshake(h) => h.timestamp,
            ControlPacket::KeepAlive(k) => k.timestamp,
            ControlPacket::Ack(a) => a.timestamp,
            ControlPacket::Nak(n) => n.timestamp,
            ControlPacket::CongestionWarning(c) => c.timestamp,
            ControlPacket::Shutdown(s) => s.timestamp,
            ControlPacket::AckAck(a) => a.timestamp,
            ControlPacket::DropReq(d) => d.timestamp,
            ControlPacket::PeerError(p) => p.timestamp,
            ControlPacket::UserDefined(u) => u.timestamp,
        }
    }

    fn dest_socket_id(&self) -> u32 {
        match self {
            ControlPacket::Handshake(h) => h.dest_socket_id,
            ControlPacket::KeepAlive(k) => k.dest_socket_id,
            ControlPacket::Ack(a) => a.dest_socket_id,
            ControlPacket::Nak(n) => n.dest_socket_id,
            ControlPacket::CongestionWarning(c) => c.dest_socket_id,
            ControlPacket::Shutdown(s) => s.dest_socket_id,
            ControlPacket::AckAck(a) => a.dest_socket_id,
            ControlPacket::DropReq(d) => d.dest_socket_id,
            ControlPacket::PeerError(p) => p.dest_socket_id,
            ControlPacket::UserDefined(u) => u.dest_socket_id,
        }
    }

    fn cif_len(&self) -> usize {
        match self {
            ControlPacket::Handshake(h) => h.cif_len(),
            ControlPacket::KeepAlive(_)
            | ControlPacket::CongestionWarning(_)
            | ControlPacket::Shutdown(_)
            | ControlPacket::AckAck(_)
            | ControlPacket::PeerError(_) => 0,
            ControlPacket::Ack(a) => a.cif_len(),
            ControlPacket::Nak(n) => n.cif_len(),
            ControlPacket::DropReq(d) => d.cif_len(),
            ControlPacket::UserDefined(u) => u.cif.len(),
        }
    }

    /// Number of bytes [`Self::serialize_into`] will write.
    pub fn serialized_len(&self) -> usize {
        SRT_HEADER_LEN + self.cif_len()
    }

    /// Serialize this control packet into `buf`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let control_type_bits = self.control_type().to_bits();
        if control_type_bits > 0x7FFF {
            return Err(Error::FieldTooWide {
                what: "Control Type",
                value: u64::from(control_type_bits),
                bits: 15,
            });
        }
        let word0 = super::F_BIT | (u32::from(control_type_bits) << 16) | u32::from(self.subtype());
        put_be32(buf, 0, word0);
        put_be32(buf, 4, self.word1());
        put_be32(buf, 8, self.timestamp());
        put_be32(buf, 12, self.dest_socket_id());
        let cif = &mut buf[SRT_HEADER_LEN..len];
        match self {
            ControlPacket::Handshake(h) => {
                h.write_cif(cif)?;
            }
            ControlPacket::KeepAlive(_)
            | ControlPacket::CongestionWarning(_)
            | ControlPacket::Shutdown(_)
            | ControlPacket::AckAck(_)
            | ControlPacket::PeerError(_) => {}
            ControlPacket::Ack(a) => {
                a.write_cif(cif)?;
            }
            ControlPacket::Nak(n) => {
                n.write_cif(cif);
            }
            ControlPacket::DropReq(d) => {
                d.write_cif(cif);
            }
            ControlPacket::UserDefined(u) => {
                cif.copy_from_slice(u.cif);
            }
        }
        Ok(len)
    }
}
