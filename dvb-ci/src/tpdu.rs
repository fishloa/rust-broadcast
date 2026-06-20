//! Transport Protocol Data Unit (TPDU) framing — ETSI EN 50221 Annex A §A.4.1,
//! Tables A.1-A.16 (PDF pp. 63-70).
//!
//! The transport layer is a command-response protocol: the host sends a Command
//! TPDU (C_TPDU), the module replies with a Response TPDU (R_TPDU) that always
//! ends in a Status Byte. The `length_field` uses the Table 1 coding (see
//! [`crate::length`]). This module covers the wire framing only — the PC-Card
//! hardware transport itself is out of scope.

use crate::error::{Error, Result};
use crate::length;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// `tpdu_tag` values — Table A.16 (p. 70). One byte each.
pub mod tags {
    /// `TSB` (status byte) = `80`.
    pub const SB: u8 = 0x80;
    /// `TRCV` (receive data) = `81`.
    pub const RCV: u8 = 0x81;
    /// `Tcreate_t_c` = `82`.
    pub const CREATE_T_C: u8 = 0x82;
    /// `Tc_t_c_reply` = `83`.
    pub const C_T_C_REPLY: u8 = 0x83;
    /// `Tdelete_t_c` = `84`.
    pub const DELETE_T_C: u8 = 0x84;
    /// `Td_t_c_reply` = `85`.
    pub const D_T_C_REPLY: u8 = 0x85;
    /// `Trequest_t_c` = `86`.
    pub const REQUEST_T_C: u8 = 0x86;
    /// `Tnew_t_c` = `87`.
    pub const NEW_T_C: u8 = 0x87;
    /// `Tt_c_error` = `88`.
    pub const T_C_ERROR: u8 = 0x88;
    /// `Tdata_last` (last data block) = `A0`.
    pub const DATA_LAST: u8 = 0xA0;
    /// `Tdata_more` (more data follows) = `A1`.
    pub const DATA_MORE: u8 = 0xA1;
}

/// Status Byte value (`SB_value`, Figure A.6 + Table A.3, p. 64). The single
/// 1-bit DA (Data Available) flag is bit 8; the remaining bits are reserved
/// (shall be zero) and preserved here for fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SbValue(pub u8);

impl SbValue {
    /// DA (Data Available): true when the module has a message for the host.
    #[must_use]
    pub const fn data_available(self) -> bool {
        self.0 & 0x80 != 0
    }
    /// Build an `SB_value` with the DA bit set/clear and reserved bits zero.
    #[must_use]
    pub const fn new(data_available: bool) -> Self {
        Self(if data_available { 0x80 } else { 0x00 })
    }
}

/// `c_TPDU_tag` for the data-carrying C_TPDU (chaining): last vs. more (§A.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DataBlock {
    /// `Tdata_last` (`A0`): the final (or only) data block.
    Last,
    /// `Tdata_more` (`A1`): more blocks follow.
    More,
}

impl DataBlock {
    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            tags::DATA_LAST => Some(Self::Last),
            tags::DATA_MORE => Some(Self::More),
            _ => None,
        }
    }
    /// The `tpdu_tag` byte for this data block (`A0` last / `A1` more).
    #[must_use]
    pub fn to_tag(self) -> u8 {
        match self {
            Self::Last => tags::DATA_LAST,
            Self::More => tags::DATA_MORE,
        }
    }
    /// Spec token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Last => "data_last",
            Self::More => "data_more",
        }
    }
}
dvb_common::impl_spec_display!(DataBlock);

// --- single-field connection-management objects (tag + length + t_c_id) ---

/// A connection-management object carrying just `t_c_id`: `Create_T_C`
/// (Table A.4), `C_T_C_Reply`, `Delete_T_C` (A.6), `D_T_C_Reply`, `Request_T_C`
/// (A.8). The `tag` distinguishes which (`length_field = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TcObject {
    /// The `tpdu_tag` (one of `CREATE_T_C`/`C_T_C_REPLY`/`DELETE_T_C`/
    /// `D_T_C_REPLY`/`REQUEST_T_C`).
    pub tag: u8,
    /// `t_c_id` — the transport connection identifier.
    pub t_c_id: u8,
}

impl<'a> Parse<'a> for TcObject {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "TcObject",
            });
        }
        let tag = bytes[0];
        let (len, hdr) = length::decode(&bytes[1..])?;
        if len != 1 {
            return Err(Error::InvalidObject {
                what: "TcObject",
                reason: "length_field must be 1",
            });
        }
        let t_c_id = *bytes.get(1 + hdr).ok_or(Error::BufferTooShort {
            need: 1 + hdr + 1,
            have: bytes.len(),
            what: "TcObject t_c_id",
        })?;
        Ok(Self { tag, t_c_id })
    }
}
impl Serialize for TcObject {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        3 // tag + length_field(1) + t_c_id
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 3 {
            return Err(Error::OutputBufferTooSmall {
                need: 3,
                have: buf.len(),
            });
        }
        buf[0] = self.tag;
        buf[1] = 1;
        buf[2] = self.t_c_id;
        Ok(3)
    }
}

/// `New_T_C` (Table A.9, `length=2`): a new connection identifier to establish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct NewTc {
    /// `t_c_id` of the connection issuing this.
    pub t_c_id: u8,
    /// `new_t_c_id` — the identifier for the new connection.
    pub new_t_c_id: u8,
}

impl<'a> Parse<'a> for NewTc {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = parse_fixed(bytes, tags::NEW_T_C, 2, "New_T_C")?;
        Ok(Self {
            t_c_id: body[0],
            new_t_c_id: body[1],
        })
    }
}
impl Serialize for NewTc {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_fixed(tags::NEW_T_C, &[self.t_c_id, self.new_t_c_id], buf)
    }
}

/// `T_C_Error` (Table A.10, `length=2`): an error on a transport connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TcError {
    /// `t_c_id`.
    pub t_c_id: u8,
    /// `error_code` (Table A.11: `1` = no transport connections available).
    pub error_code: u8,
}

impl<'a> Parse<'a> for TcError {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = parse_fixed(bytes, tags::T_C_ERROR, 2, "T_C_Error")?;
        Ok(Self {
            t_c_id: body[0],
            error_code: body[1],
        })
    }
}
impl Serialize for TcError {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_fixed(tags::T_C_ERROR, &[self.t_c_id, self.error_code], buf)
    }
}

/// `C_TPDU` (Table A.1): a Command TPDU, host → module. `length_field` covers
/// `t_c_id` + data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommandTpdu<'a> {
    /// The `c_tpdu_tag` (e.g. `RCV`, or `DATA_LAST`/`DATA_MORE` for Send Data).
    pub tag: u8,
    /// `t_c_id`.
    pub t_c_id: u8,
    /// Data field (`length_value - 1` bytes).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub data: &'a [u8],
}

impl<'a> Parse<'a> for CommandTpdu<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "C_TPDU",
            });
        }
        let tag = bytes[0];
        let (len, hdr) = length::decode(&bytes[1..])?;
        if len == 0 {
            return Err(Error::InvalidObject {
                what: "C_TPDU",
                reason: "length_field must include t_c_id (>=1)",
            });
        }
        let start = 1 + hdr;
        let end = start + len;
        if bytes.len() < end {
            return Err(Error::LengthMismatch {
                what: "C_TPDU",
                declared: len,
                actual: bytes.len().saturating_sub(start),
            });
        }
        Ok(Self {
            tag,
            t_c_id: bytes[start],
            data: &bytes[start + 1..end],
        })
    }
}
impl Serialize for CommandTpdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let len_value = 1 + self.data.len();
        1 + length::encoded_len(len_value) + len_value
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len_value = 1 + self.data.len();
        let total = 1 + length::encoded_len(len_value) + len_value;
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        buf[0] = self.tag;
        let mut pos = 1 + length::encode_into(len_value, &mut buf[1..])?;
        buf[pos] = self.t_c_id;
        pos += 1;
        buf[pos..pos + self.data.len()].copy_from_slice(self.data);
        Ok(pos + self.data.len())
    }
}

/// `R_TPDU` (Table A.2): a Response TPDU, module → host. The mandatory trailing
/// Status (SB) is NOT included in the `length_field` and is modelled separately.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ResponseTpdu<'a> {
    /// The `r_tpdu_tag` (e.g. `DATA_LAST`/`DATA_MORE`).
    pub tag: u8,
    /// `t_c_id`.
    pub t_c_id: u8,
    /// Data field (`length_value - 1` bytes).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub data: &'a [u8],
    /// `SB_value` from the mandatory trailing Status.
    pub sb_value: SbValue,
    /// Whether the data block was the last (`Tdata_last`) — convenience view of
    /// `tag` for chaining, `None` if `tag` is not a data block tag.
    pub block: Option<DataBlock>,
}

impl<'a> Parse<'a> for ResponseTpdu<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "R_TPDU",
            });
        }
        let tag = bytes[0];
        let (len, hdr) = length::decode(&bytes[1..])?;
        if len == 0 {
            return Err(Error::InvalidObject {
                what: "R_TPDU",
                reason: "length_field must include t_c_id (>=1)",
            });
        }
        let start = 1 + hdr;
        let data_end = start + len;
        // Status trailer: SB_tag + length_field(=2) + t_c_id + SB_value = 4 bytes.
        let status_end = data_end + 4;
        if bytes.len() < status_end {
            return Err(Error::BufferTooShort {
                need: status_end,
                have: bytes.len(),
                what: "R_TPDU status",
            });
        }
        if bytes[data_end] != tags::SB {
            return Err(Error::UnexpectedTpduTag {
                got: bytes[data_end],
                expected: tags::SB,
                what: "R_TPDU SB_tag",
            });
        }
        if bytes[data_end + 1] != 2 {
            return Err(Error::InvalidObject {
                what: "R_TPDU status",
                reason: "SB length_field must be 2",
            });
        }
        // bytes[data_end+2] = t_c_id (status), bytes[data_end+3] = SB_value.
        Ok(Self {
            tag,
            t_c_id: bytes[start],
            data: &bytes[start + 1..data_end],
            sb_value: SbValue(bytes[data_end + 3]),
            block: DataBlock::from_tag(tag),
        })
    }
}
impl Serialize for ResponseTpdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let len_value = 1 + self.data.len();
        1 + length::encoded_len(len_value) + len_value + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let len_value = 1 + self.data.len();
        buf[0] = self.tag;
        let mut pos = 1 + length::encode_into(len_value, &mut buf[1..])?;
        buf[pos] = self.t_c_id;
        pos += 1;
        buf[pos..pos + self.data.len()].copy_from_slice(self.data);
        pos += self.data.len();
        // Status trailer.
        buf[pos] = tags::SB;
        buf[pos + 1] = 2;
        buf[pos + 2] = self.t_c_id;
        buf[pos + 3] = self.sb_value.0;
        Ok(pos + 4)
    }
}

// --- helpers for the fixed-length tag+length+body objects ---

fn parse_fixed<'a>(
    bytes: &'a [u8],
    expected: u8,
    body_len: usize,
    what: &'static str,
) -> Result<&'a [u8]> {
    if bytes.is_empty() {
        return Err(Error::BufferTooShort {
            need: 1,
            have: 0,
            what,
        });
    }
    if bytes[0] != expected {
        return Err(Error::UnexpectedTpduTag {
            got: bytes[0],
            expected,
            what,
        });
    }
    let (len, hdr) = length::decode(&bytes[1..])?;
    if len != body_len {
        return Err(Error::InvalidObject {
            what,
            reason: "unexpected length_field",
        });
    }
    let start = 1 + hdr;
    let end = start + body_len;
    if bytes.len() < end {
        return Err(Error::BufferTooShort {
            need: end,
            have: bytes.len(),
            what,
        });
    }
    Ok(&bytes[start..end])
}

fn write_fixed(tag: u8, body: &[u8], buf: &mut [u8]) -> Result<usize> {
    let total = 2 + body.len();
    if buf.len() < total {
        return Err(Error::OutputBufferTooSmall {
            need: total,
            have: buf.len(),
        });
    }
    buf[0] = tag;
    buf[1] = body.len() as u8;
    buf[2..2 + body.len()].copy_from_slice(body);
    Ok(total)
}

/// Build a `Create_T_C` object (`tag = CREATE_T_C`, `t_c_id`).
#[must_use]
pub fn create_t_c(t_c_id: u8) -> TcObject {
    TcObject {
        tag: tags::CREATE_T_C,
        t_c_id,
    }
}

/// Collect a `Vec` of the wire bytes for a [`TcObject`] (convenience).
#[must_use]
pub fn tc_object_bytes(o: &TcObject) -> Vec<u8> {
    o.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc_object_round_trip() {
        let o = create_t_c(0x01);
        let bytes = o.to_bytes();
        assert_eq!(bytes, [0x82, 0x01, 0x01]);
        assert_eq!(TcObject::parse(&bytes).unwrap(), o);
    }

    #[test]
    fn new_tc_round_trip() {
        let n = NewTc {
            t_c_id: 1,
            new_t_c_id: 2,
        };
        let bytes = n.to_bytes();
        assert_eq!(bytes, [0x87, 0x02, 0x01, 0x02]);
        assert_eq!(NewTc::parse(&bytes).unwrap(), n);
    }

    #[test]
    fn tc_error_round_trip() {
        let e = TcError {
            t_c_id: 3,
            error_code: 1,
        };
        let bytes = e.to_bytes();
        assert_eq!(bytes, [0x88, 0x02, 0x03, 0x01]);
        assert_eq!(TcError::parse(&bytes).unwrap(), e);
    }

    #[test]
    fn command_tpdu_round_trip() {
        let c = CommandTpdu {
            tag: tags::DATA_LAST,
            t_c_id: 1,
            data: &[0xAA, 0xBB, 0xCC],
        };
        let bytes = c.to_bytes();
        // tag A0, length 4 (t_c_id + 3 data), t_c_id, data.
        assert_eq!(bytes, [0xA0, 0x04, 0x01, 0xAA, 0xBB, 0xCC]);
        assert_eq!(CommandTpdu::parse(&bytes).unwrap(), c);
    }

    #[test]
    fn receive_data_command_no_payload() {
        let c = CommandTpdu {
            tag: tags::RCV,
            t_c_id: 1,
            data: &[],
        };
        let bytes = c.to_bytes();
        assert_eq!(bytes, [0x81, 0x01, 0x01]);
        assert_eq!(CommandTpdu::parse(&bytes).unwrap(), c);
    }

    #[test]
    fn response_tpdu_round_trip_with_status() {
        let r = ResponseTpdu {
            tag: tags::DATA_LAST,
            t_c_id: 1,
            data: &[0x9F, 0x80, 0x30, 0x00],
            sb_value: SbValue::new(true),
            block: Some(DataBlock::Last),
        };
        let bytes = r.to_bytes();
        // header: A0 05 01 + 4 data ; status: 80 02 01 80.
        assert_eq!(
            bytes,
            [0xA0, 0x05, 0x01, 0x9F, 0x80, 0x30, 0x00, 0x80, 0x02, 0x01, 0x80]
        );
        let parsed = ResponseTpdu::parse(&bytes).unwrap();
        assert_eq!(parsed, r);
        assert!(parsed.sb_value.data_available());
        assert_eq!(parsed.block, Some(DataBlock::Last));
    }

    #[test]
    fn mutating_data_changes_bytes() {
        let c = CommandTpdu {
            tag: tags::DATA_LAST,
            t_c_id: 1,
            data: &[0xAA],
        };
        let a = c.to_bytes();
        let b = CommandTpdu {
            tag: tags::DATA_LAST,
            t_c_id: 1,
            data: &[0xBB],
        }
        .to_bytes();
        assert_ne!(a, b);
    }

    #[test]
    fn sb_value_da() {
        assert!(SbValue::new(true).data_available());
        assert!(!SbValue::new(false).data_available());
    }
}
