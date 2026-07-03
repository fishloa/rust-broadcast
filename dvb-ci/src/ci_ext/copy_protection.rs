//! Copy Protection objects — ETSI TS 101 699 V1.1.1 §6.6, Tables 69-73
//! (PDF pp. 62-63). See `docs/ci_plus/copy-protection.md`.
//!
//! Resource ID `0x00041ii1` (`ii` = Module ID). Generic control of a host's
//! copy-protection function; the detailed semantics are CP-system-specific.
//!
//! - `cp_query` (`9F 80 00`, Table 69) — app → host: query status of a CP system.
//! - `cp_reply` (`9F 80 01`, Table 70) — host → app: status reply.
//! - `cp_command` (`9F 80 02`, Table 72) — app → host: opaque command bytes.
//! - `cp_response` (`9F 80 03`, Table 73) — host → app: opaque response bytes.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for Copy Protection (Tables 69-73).
pub mod tag {
    use crate::tag::ApduTag;
    /// `CopyProtectionQueryTag` = `9F 80 00`.
    pub const CP_QUERY: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `CPReplyTag` = `9F 80 01`.
    pub const CP_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `CPCommandTag` = `9F 80 02`.
    pub const CP_COMMAND: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `CPResponseTag` = `9F 80 03`.
    pub const CP_RESPONSE: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
}

/// `Status` — copy-protection status (Table 71).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CpStatus {
    /// `01` — Copy Protection Inactive.
    Inactive,
    /// `02` — Copy Protection Active.
    Active,
    /// `FF` — ID mismatch.
    IdMismatch,
    /// Any other value (reserved).
    Reserved(u8),
}

impl CpStatus {
    /// Decode the `Status` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Inactive,
            0x02 => Self::Active,
            0xFF => Self::IdMismatch,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Inactive => 0x01,
            Self::Active => 0x02,
            Self::IdMismatch => 0xFF,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Inactive => "Copy Protection Inactive",
            Self::Active => "Copy Protection Active",
            Self::IdMismatch => "ID mismatch",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(CpStatus, Reserved);

/// `cp_query()` (Table 69): app → host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpQuery {
    /// 24-bit `CopyProtectionID` (an IEEE `company_id`).
    pub copy_protection_id: u32,
}

/// `cp_reply()` (Table 70): host → app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpReply {
    /// 24-bit `CopyProtectionID`.
    pub copy_protection_id: u32,
    /// `Status`.
    pub status: CpStatus,
}

/// `cp_command()` (Table 72): app → host. The `cp_command_byte`s are
/// **opaque, CP-system-specific** bytes carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpCommand<'a> {
    /// 24-bit `CopyProtectionID`.
    pub copy_protection_id: u32,
    /// Opaque command bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub command_bytes: &'a [u8],
}

/// `cp_response()` (Table 73): host → app. Identical to [`CpCommand`] except for
/// the tag; the `cp_response_byte`s are opaque CP-system-specific bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CpResponse<'a> {
    /// 24-bit `CopyProtectionID`.
    pub copy_protection_id: u32,
    /// Opaque response bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub response_bytes: &'a [u8],
}

/// Width of the `CopyProtectionID` field (24 bits).
const CP_ID_LEN: usize = 3;

fn read_cp_id(body: &[u8]) -> u32 {
    ((body[0] as u32) << 16) | ((body[1] as u32) << 8) | body[2] as u32
}

fn write_cp_id(id: u32, buf: &mut [u8]) {
    buf[0] = (id >> 16) as u8;
    buf[1] = (id >> 8) as u8;
    buf[2] = id as u8;
}

// --- cp_query ---

impl<'a> Parse<'a> for CpQuery {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CP_QUERY, "cp_query")?;
        if body.len() < CP_ID_LEN {
            return Err(Error::BufferTooShort {
                need: CP_ID_LEN,
                have: body.len(),
                what: "cp_query",
            });
        }
        Ok(Self {
            copy_protection_id: read_cp_id(body),
        })
    }
}
impl Serialize for CpQuery {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CP_ID_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::CP_QUERY, CP_ID_LEN, buf)?;
        write_cp_id(self.copy_protection_id, &mut buf[pos..]);
        Ok(pos + CP_ID_LEN)
    }
}

// --- cp_reply ---

// CopyProtectionID(3) + Status(1).
const CP_REPLY_BODY: usize = CP_ID_LEN + 1;

impl<'a> Parse<'a> for CpReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CP_REPLY, "cp_reply")?;
        if body.len() < CP_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: CP_REPLY_BODY,
                have: body.len(),
                what: "cp_reply",
            });
        }
        Ok(Self {
            copy_protection_id: read_cp_id(body),
            status: CpStatus::from_u8(body[CP_ID_LEN]),
        })
    }
}
impl Serialize for CpReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CP_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::CP_REPLY, CP_REPLY_BODY, buf)?;
        write_cp_id(self.copy_protection_id, &mut buf[pos..]);
        buf[pos + CP_ID_LEN] = self.status.to_u8();
        Ok(pos + CP_REPLY_BODY)
    }
}

// --- cp_command ---

impl<'a> Parse<'a> for CpCommand<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CP_COMMAND, "cp_command")?;
        if body.len() < CP_ID_LEN {
            return Err(Error::BufferTooShort {
                need: CP_ID_LEN,
                have: body.len(),
                what: "cp_command",
            });
        }
        Ok(Self {
            copy_protection_id: read_cp_id(body),
            command_bytes: &body[CP_ID_LEN..],
        })
    }
}
impl Serialize for CpCommand<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CP_ID_LEN + self.command_bytes.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = CP_ID_LEN + self.command_bytes.len();
        let mut pos = objects::write_apdu_header(tag::CP_COMMAND, body_len, buf)?;
        write_cp_id(self.copy_protection_id, &mut buf[pos..]);
        pos += CP_ID_LEN;
        buf[pos..pos + self.command_bytes.len()].copy_from_slice(self.command_bytes);
        Ok(pos + self.command_bytes.len())
    }
}

// --- cp_response ---

impl<'a> Parse<'a> for CpResponse<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CP_RESPONSE, "cp_response")?;
        if body.len() < CP_ID_LEN {
            return Err(Error::BufferTooShort {
                need: CP_ID_LEN,
                have: body.len(),
                what: "cp_response",
            });
        }
        Ok(Self {
            copy_protection_id: read_cp_id(body),
            response_bytes: &body[CP_ID_LEN..],
        })
    }
}
impl Serialize for CpResponse<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CP_ID_LEN + self.response_bytes.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = CP_ID_LEN + self.response_bytes.len();
        let mut pos = objects::write_apdu_header(tag::CP_RESPONSE, body_len, buf)?;
        write_cp_id(self.copy_protection_id, &mut buf[pos..]);
        pos += CP_ID_LEN;
        buf[pos..pos + self.response_bytes.len()].copy_from_slice(self.response_bytes);
        Ok(pos + self.response_bytes.len())
    }
}

/// Resource-scoped dispatch over the Copy Protection objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CopyProtectionApdu<'a> {
    /// `cp_query` (`9F 80 00`).
    CpQuery(CpQuery),
    /// `cp_reply` (`9F 80 01`).
    CpReply(CpReply),
    /// `cp_command` (`9F 80 02`).
    CpCommand(CpCommand<'a>),
    /// `cp_response` (`9F 80 03`).
    CpResponse(CpResponse<'a>),
}

impl<'a> CopyProtectionApdu<'a> {
    /// Parse a Copy Protection APDU, dispatching on the `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "copy_protection apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::CP_QUERY => Ok(Self::CpQuery(CpQuery::parse(body)?)),
            tag::CP_REPLY => Ok(Self::CpReply(CpReply::parse(body)?)),
            tag::CP_COMMAND => Ok(Self::CpCommand(CpCommand::parse(body)?)),
            tag::CP_RESPONSE => Ok(Self::CpResponse(CpResponse::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::CP_QUERY.as_u24(),
                what: "copy_protection",
            }),
        }
    }
}

impl Serialize for CopyProtectionApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CpQuery(o) => o.serialized_len(),
            Self::CpReply(o) => o.serialized_len(),
            Self::CpCommand(o) => o.serialized_len(),
            Self::CpResponse(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CpQuery(o) => o.serialize_into(buf),
            Self::CpReply(o) => o.serialize_into(buf),
            Self::CpCommand(o) => o.serialize_into(buf),
            Self::CpResponse(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp_query_round_trips_and_bites() {
        let q = CpQuery {
            copy_protection_id: 0xAABBCC,
        };
        let bytes = q.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x03, 0xAA, 0xBB, 0xCC]);
        assert_eq!(CpQuery::parse(&bytes).unwrap(), q);
        let other = CpQuery {
            copy_protection_id: 0xAABBCD,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn cp_reply_round_trips_and_bites() {
        let r = CpReply {
            copy_protection_id: 0x010203,
            status: CpStatus::Active,
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x04, 0x01, 0x02, 0x03, 0x02]);
        assert_eq!(CpReply::parse(&bytes).unwrap(), r);
        assert_eq!(r.status.name(), "Copy Protection Active");
        let mut other = r;
        other.status = CpStatus::IdMismatch;
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[7], 0xFF);
    }

    #[test]
    fn cp_command_round_trips_and_bites() {
        let c = CpCommand {
            copy_protection_id: 0x112233,
            command_bytes: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let bytes = c.to_bytes();
        // tag(3) + len(1) + id(3) + 4 = 11; body len = 7 = 0x07.
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x02, 0x07, 0x11, 0x22, 0x33, 0xDE, 0xAD, 0xBE, 0xEF
            ]
        );
        assert_eq!(CpCommand::parse(&bytes).unwrap(), c);
        let other = CpCommand {
            copy_protection_id: 0x112233,
            command_bytes: &[0xDE, 0xAD, 0xBE, 0x00],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn cp_response_round_trips() {
        let r = CpResponse {
            copy_protection_id: 0x445566,
            response_bytes: &[0x01, 0x02],
        };
        let bytes = r.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x03, 0x05, 0x44, 0x55, 0x66, 0x01, 0x02]
        );
        assert_eq!(CpResponse::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let q = CpQuery {
            copy_protection_id: 0,
        }
        .to_bytes();
        assert!(matches!(
            CopyProtectionApdu::parse(&q).unwrap(),
            CopyProtectionApdu::CpQuery(_)
        ));
        let resp = CpResponse {
            copy_protection_id: 0x1,
            response_bytes: &[0xFF],
        }
        .to_bytes();
        let parsed = CopyProtectionApdu::parse(&resp).unwrap();
        assert!(matches!(parsed, CopyProtectionApdu::CpResponse(_)));
        assert_eq!(parsed.to_bytes(), resp);
    }
}
