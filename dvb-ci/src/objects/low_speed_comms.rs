//! Low-Speed Communications objects — ETSI EN 50221 §8.7.1, Tables 52-56
//! (PDF pp. 51-54).
//!
//! - `comms_cmd` (`9F 8C 00`, Table 52) — connect / disconnect / set-params /
//!   enquire-status / get-next-buffer.
//! - `connection_descriptor` (`9F 8C 01`, Table 53) — telephone / cable-return
//!   connection info; also nested inside Connect_on_Channel.
//! - `comms_reply` (`9F 8C 02`, Table 54) — id + return_value.
//! - `comms_send` (`9F 8C 03` last / `9F 8C 04` more, Table 55) — phase + bytes.
//! - `comms_rcv` (`9F 8C 05` last / `9F 8C 06` more, Table 56) — phase + bytes.

use crate::error::{Error, Result};
use crate::length;
use crate::tag::{self, ApduTag};
use crate::traits::ApduDef;
use dvb_common::{Parse, Serialize};

/// `comms_command_id` values (Table 52, p. 52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CommsCommandId {
    /// `01` — establish communication on the comms resource.
    ConnectOnChannel,
    /// `02` — terminate the connection.
    DisconnectOnChannel,
    /// `03` — set buffer size / timeout.
    SetParams,
    /// `04` — request the current connection status.
    EnquireStatus,
    /// `05` — receive-side flow control.
    GetNextBuffer,
    /// Any other value (reserved).
    Reserved(u8),
}

impl CommsCommandId {
    /// Decode a `comms_command_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::ConnectOnChannel,
            0x02 => Self::DisconnectOnChannel,
            0x03 => Self::SetParams,
            0x04 => Self::EnquireStatus,
            0x05 => Self::GetNextBuffer,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::ConnectOnChannel => 0x01,
            Self::DisconnectOnChannel => 0x02,
            Self::SetParams => 0x03,
            Self::EnquireStatus => 0x04,
            Self::GetNextBuffer => 0x05,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConnectOnChannel => "Connect_on_Channel",
            Self::DisconnectOnChannel => "Disconnect_on_Channel",
            Self::SetParams => "Set_Params",
            Self::EnquireStatus => "Enquire_Status",
            Self::GetNextBuffer => "Get_Next_Buffer",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(CommsCommandId, Reserved);

/// `connection_descriptor_type` values (Table 53, p. 52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ConnectionDescriptorType {
    /// `01` — an SI telephone_descriptor() follows.
    SiTelephoneDescriptor,
    /// `02` — a cable-return-channel `channel_id` follows.
    CableReturnChannelDescriptor,
    /// Any other value (reserved).
    Reserved(u8),
}

impl ConnectionDescriptorType {
    /// Decode a `connection_descriptor_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::SiTelephoneDescriptor,
            0x02 => Self::CableReturnChannelDescriptor,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::SiTelephoneDescriptor => 0x01,
            Self::CableReturnChannelDescriptor => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::SiTelephoneDescriptor => "SI_Telephone_Descriptor",
            Self::CableReturnChannelDescriptor => "Cable_Return_Channel_Descriptor",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ConnectionDescriptorType, Reserved);

/// `connection_descriptor()` object (Table 53): carries the connection info.
///
/// The body after the `connection_descriptor_type` byte is branch-dependent (an
/// SI `telephone_descriptor()` whose internal layout EN 50221 does not reproduce,
/// or a 1-byte `channel_id`); it is carried verbatim so it round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConnectionDescriptor<'a> {
    /// `connection_descriptor_type`.
    pub descriptor_type: ConnectionDescriptorType,
    /// The branch-dependent payload (telephone_descriptor bytes, or the
    /// `channel_id` byte), verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub payload: &'a [u8],
}

impl<'a> ConnectionDescriptor<'a> {
    /// Parse a nested `connection_descriptor()` at the front of `bytes`, returning
    /// the object and how many bytes it consumed (tag + length + body).
    pub(crate) fn parse_component(bytes: &'a [u8]) -> Result<(Self, usize)> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "connection_descriptor tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        if t != tag::CONNECTION_DESCRIPTOR {
            return Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::CONNECTION_DESCRIPTOR.as_u24(),
                what: "connection_descriptor",
            });
        }
        let (len_value, len_hdr) = length::decode(&bytes[3..])?;
        let body_start = 3 + len_hdr;
        let body_end = body_start + len_value;
        if bytes.len() < body_end {
            return Err(Error::LengthMismatch {
                what: "connection_descriptor",
                declared: len_value,
                actual: bytes.len().saturating_sub(body_start),
            });
        }
        let body = &bytes[body_start..body_end];
        let type_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "connection_descriptor type",
        })?;
        Ok((
            Self {
                descriptor_type: ConnectionDescriptorType::from_u8(type_byte),
                payload: &body[1..],
            },
            body_end,
        ))
    }

    pub(crate) fn component_len(&self) -> usize {
        super::apdu_len(1 + self.payload.len())
    }

    pub(crate) fn serialize_component(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.payload.len();
        let mut pos = super::write_apdu_header(tag::CONNECTION_DESCRIPTOR, body_len, buf)?;
        buf[pos] = self.descriptor_type.to_u8();
        pos += 1;
        buf[pos..pos + self.payload.len()].copy_from_slice(self.payload);
        pos += self.payload.len();
        Ok(pos)
    }
}

impl<'a> Parse<'a> for ConnectionDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (c, consumed) = Self::parse_component(bytes)?;
        if consumed != bytes.len() {
            return Err(Error::LengthMismatch {
                what: "connection_descriptor",
                declared: consumed,
                actual: bytes.len(),
            });
        }
        Ok(c)
    }
}

impl Serialize for ConnectionDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.component_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_component(buf)
    }
}

impl<'a> ApduDef<'a> for ConnectionDescriptor<'a> {
    const TAG: ApduTag = tag::CONNECTION_DESCRIPTOR;
    const NAME: &'static str = "CONNECTION_DESCRIPTOR";
}

/// The branch-dependent parameters of a `comms_cmd()` (Table 52).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CommsCmdParams<'a> {
    /// `Connect_on_Channel`: connection descriptor + retry_count + timeout.
    Connect {
        /// The nested `connection_descriptor()`.
        #[cfg_attr(feature = "serde", serde(borrow))]
        connection_descriptor: ConnectionDescriptor<'a>,
        /// `retry_count`.
        retry_count: u8,
        /// `timeout` (seconds).
        timeout: u8,
    },
    /// `Set_Params`: buffer_size + timeout.
    SetParams {
        /// `buffer_size` (1..=254).
        buffer_size: u8,
        /// `timeout` (units of 10 ms).
        timeout: u8,
    },
    /// `Get_Next_Buffer`: comms_phase_id.
    GetNextBuffer {
        /// `comms_phase_id` (alternates 0,1,…).
        comms_phase_id: u8,
    },
    /// No branch parameters (`Disconnect_on_Channel`, `Enquire_Status`, or a
    /// reserved command id).
    None,
}

/// `comms_cmd()` object (Table 52).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsCmd<'a> {
    /// `comms_command_id`.
    pub command_id: CommsCommandId,
    /// The branch-dependent parameters keyed by `command_id`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub params: CommsCmdParams<'a>,
}

impl<'a> Parse<'a> for CommsCmd<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::COMMS_CMD, "comms_cmd")?;
        let id_byte = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "comms_cmd command_id",
        })?;
        let command_id = CommsCommandId::from_u8(id_byte);
        let rest = &body[1..];
        let params = match command_id {
            CommsCommandId::ConnectOnChannel => {
                let (cd, consumed) = ConnectionDescriptor::parse_component(rest)?;
                let tail = &rest[consumed..];
                if tail.len() < 2 {
                    return Err(Error::BufferTooShort {
                        need: 2,
                        have: tail.len(),
                        what: "comms_cmd connect retry/timeout",
                    });
                }
                CommsCmdParams::Connect {
                    connection_descriptor: cd,
                    retry_count: tail[0],
                    timeout: tail[1],
                }
            }
            CommsCommandId::SetParams => {
                if rest.len() < 2 {
                    return Err(Error::BufferTooShort {
                        need: 2,
                        have: rest.len(),
                        what: "comms_cmd set_params",
                    });
                }
                CommsCmdParams::SetParams {
                    buffer_size: rest[0],
                    timeout: rest[1],
                }
            }
            CommsCommandId::GetNextBuffer => {
                let comms_phase_id = *rest.first().ok_or(Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "comms_cmd get_next_buffer",
                })?;
                CommsCmdParams::GetNextBuffer { comms_phase_id }
            }
            _ => CommsCmdParams::None,
        };
        Ok(Self { command_id, params })
    }
}

impl<'a> CommsCmd<'a> {
    fn body_len(&self) -> usize {
        1 + match &self.params {
            CommsCmdParams::Connect {
                connection_descriptor,
                ..
            } => connection_descriptor.component_len() + 2,
            CommsCmdParams::SetParams { .. } => 2,
            CommsCmdParams::GetNextBuffer { .. } => 1,
            CommsCmdParams::None => 0,
        }
    }
}

impl Serialize for CommsCmd<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(self.body_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.body_len();
        let mut pos = super::write_apdu_header(tag::COMMS_CMD, body_len, buf)?;
        buf[pos] = self.command_id.to_u8();
        pos += 1;
        match &self.params {
            CommsCmdParams::Connect {
                connection_descriptor,
                retry_count,
                timeout,
            } => {
                pos += connection_descriptor.serialize_component(&mut buf[pos..])?;
                buf[pos] = *retry_count;
                buf[pos + 1] = *timeout;
                pos += 2;
            }
            CommsCmdParams::SetParams {
                buffer_size,
                timeout,
            } => {
                buf[pos] = *buffer_size;
                buf[pos + 1] = *timeout;
                pos += 2;
            }
            CommsCmdParams::GetNextBuffer { comms_phase_id } => {
                buf[pos] = *comms_phase_id;
                pos += 1;
            }
            CommsCmdParams::None => {}
        }
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for CommsCmd<'a> {
    const TAG: ApduTag = tag::COMMS_CMD;
    const NAME: &'static str = "COMMS_CMD";
}

/// `comms_reply_id` values (Table 54, p. 53).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CommsReplyId {
    /// `01` — Connect acknowledgement.
    ConnectAck,
    /// `02` — Disconnect acknowledgement.
    DisconnectAck,
    /// `03` — Set Params acknowledgement.
    SetParamsAck,
    /// `04` — Status reply (return_value 0 = Disconnected, 1 = Connected).
    StatusReply,
    /// `05` — Get Next Buffer acknowledgement.
    GetNextBufferAck,
    /// `06` — Send acknowledgement (return_value = acknowledged phase).
    SendAck,
    /// Any other value (reserved).
    Reserved(u8),
}

impl CommsReplyId {
    /// Decode a `comms_reply_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::ConnectAck,
            0x02 => Self::DisconnectAck,
            0x03 => Self::SetParamsAck,
            0x04 => Self::StatusReply,
            0x05 => Self::GetNextBufferAck,
            0x06 => Self::SendAck,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::ConnectAck => 0x01,
            Self::DisconnectAck => 0x02,
            Self::SetParamsAck => 0x03,
            Self::StatusReply => 0x04,
            Self::GetNextBufferAck => 0x05,
            Self::SendAck => 0x06,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ConnectAck => "Connect_Ack",
            Self::DisconnectAck => "Disconnect_Ack",
            Self::SetParamsAck => "Set_Params_Ack",
            Self::StatusReply => "Status_Reply",
            Self::GetNextBufferAck => "Get_Next_Buffer_Ack",
            Self::SendAck => "Send_Ack",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(CommsReplyId, Reserved);

/// `comms_reply()` object (Table 54): id + signed return_value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsReply {
    /// `comms_reply_id`.
    pub reply_id: CommsReplyId,
    /// `return_value` — positive/zero = OK, negative = error (carried as raw u8).
    pub return_value: u8,
}

// comms_reply_id(1) + return_value(1).
const COMMS_REPLY_BODY: usize = 2;

impl<'a> Parse<'a> for CommsReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = super::parse_apdu_header(bytes, tag::COMMS_REPLY, "comms_reply")?;
        if body.len() < COMMS_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: COMMS_REPLY_BODY,
                have: body.len(),
                what: "comms_reply",
            });
        }
        Ok(Self {
            reply_id: CommsReplyId::from_u8(body[0]),
            return_value: body[1],
        })
    }
}

impl Serialize for CommsReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(COMMS_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pos = super::write_apdu_header(tag::COMMS_REPLY, COMMS_REPLY_BODY, buf)?;
        buf[pos] = self.reply_id.to_u8();
        buf[pos + 1] = self.return_value;
        pos += COMMS_REPLY_BODY;
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for CommsReply {
    const TAG: ApduTag = tag::COMMS_REPLY;
    const NAME: &'static str = "COMMS_REPLY";
}

/// `comms_send()` object (Table 55): a `comms_phase_id` + message bytes.
///
/// The `_last` (`9F 8C 03`) / `_more` (`9F 8C 04`) tags share this body;
/// [`CommsSend::more`] selects which tag is written and is set from the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsSend<'a> {
    /// `true` = `comms_send_more`; `false` = `comms_send_last`.
    pub more: bool,
    /// `comms_phase_id` (0 or 1).
    pub comms_phase_id: u8,
    /// `message_byte`s (max 254).
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub message: &'a [u8],
}

impl<'a> CommsSend<'a> {
    /// The `apdu_tag` for this object given [`CommsSend::more`].
    #[must_use]
    pub fn tag(&self) -> ApduTag {
        if self.more {
            tag::COMMS_SEND_MORE
        } else {
            tag::COMMS_SEND_LAST
        }
    }
}

impl<'a> Parse<'a> for CommsSend<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "comms_send tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let (expected, more) = match t {
            tag::COMMS_SEND_MORE => (tag::COMMS_SEND_MORE, true),
            _ => (tag::COMMS_SEND_LAST, false),
        };
        let body = super::parse_apdu_header(bytes, expected, "comms_send")?;
        let comms_phase_id = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "comms_send comms_phase_id",
        })?;
        Ok(Self {
            more,
            comms_phase_id,
            message: &body[1..],
        })
    }
}

impl Serialize for CommsSend<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1 + self.message.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.message.len();
        let mut pos = super::write_apdu_header(self.tag(), body_len, buf)?;
        buf[pos] = self.comms_phase_id;
        pos += 1;
        buf[pos..pos + self.message.len()].copy_from_slice(self.message);
        pos += self.message.len();
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for CommsSend<'a> {
    const TAG: ApduTag = tag::COMMS_SEND_LAST;
    const NAME: &'static str = "COMMS_SEND";
}

/// `comms_rcv()` object (Table 56): a `comms_phase_id` + message bytes.
///
/// The `_last` (`9F 8C 05`) / `_more` (`9F 8C 06`) tags share this body;
/// [`CommsRcv::more`] selects which tag is written and is set from the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CommsRcv<'a> {
    /// `true` = `comms_rcv_more`; `false` = `comms_rcv_last`.
    pub more: bool,
    /// `comms_phase_id`.
    pub comms_phase_id: u8,
    /// `message_byte`s.
    #[cfg_attr(feature = "serde", serde(borrow, with = "super::bytes_serde"))]
    pub message: &'a [u8],
}

impl<'a> CommsRcv<'a> {
    /// The `apdu_tag` for this object given [`CommsRcv::more`].
    #[must_use]
    pub fn tag(&self) -> ApduTag {
        if self.more {
            tag::COMMS_RCV_MORE
        } else {
            tag::COMMS_RCV_LAST
        }
    }
}

impl<'a> Parse<'a> for CommsRcv<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "comms_rcv tag",
            });
        }
        let t = ApduTag::from_bytes(bytes[0], bytes[1], bytes[2]);
        let (expected, more) = match t {
            tag::COMMS_RCV_MORE => (tag::COMMS_RCV_MORE, true),
            _ => (tag::COMMS_RCV_LAST, false),
        };
        let body = super::parse_apdu_header(bytes, expected, "comms_rcv")?;
        let comms_phase_id = *body.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "comms_rcv comms_phase_id",
        })?;
        Ok(Self {
            more,
            comms_phase_id,
            message: &body[1..],
        })
    }
}

impl Serialize for CommsRcv<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        super::apdu_len(1 + self.message.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.message.len();
        let mut pos = super::write_apdu_header(self.tag(), body_len, buf)?;
        buf[pos] = self.comms_phase_id;
        pos += 1;
        buf[pos..pos + self.message.len()].copy_from_slice(self.message);
        pos += self.message.len();
        Ok(pos)
    }
}

impl<'a> ApduDef<'a> for CommsRcv<'a> {
    const TAG: ApduTag = tag::COMMS_RCV_LAST;
    const NAME: &'static str = "COMMS_RCV";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comms_cmd_connect_round_trips_and_bites() {
        let cmd = CommsCmd {
            command_id: CommsCommandId::ConnectOnChannel,
            params: CommsCmdParams::Connect {
                connection_descriptor: ConnectionDescriptor {
                    descriptor_type: ConnectionDescriptorType::CableReturnChannelDescriptor,
                    payload: &[0x07], // channel_id
                },
                retry_count: 3,
                timeout: 30,
            },
        };
        let bytes = cmd.to_bytes();
        // 9F8C00, body: id(01) + CD[9F8C01 len2 type02 chan07] + retry03 + to1E
        assert_eq!(
            bytes,
            [0x9F, 0x8C, 0x00, 0x09, 0x01, 0x9F, 0x8C, 0x01, 0x02, 0x02, 0x07, 0x03, 0x1E]
        );
        assert_eq!(CommsCmd::parse(&bytes).unwrap(), cmd);

        let mut other = cmd.clone();
        other.params = CommsCmdParams::Connect {
            connection_descriptor: ConnectionDescriptor {
                descriptor_type: ConnectionDescriptorType::CableReturnChannelDescriptor,
                payload: &[0x08],
            },
            retry_count: 3,
            timeout: 30,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn comms_cmd_set_params_and_get_next() {
        let sp = CommsCmd {
            command_id: CommsCommandId::SetParams,
            params: CommsCmdParams::SetParams {
                buffer_size: 254,
                timeout: 10,
            },
        };
        let bytes = sp.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x00, 0x03, 0x03, 0xFE, 0x0A]);
        assert_eq!(CommsCmd::parse(&bytes).unwrap(), sp);
        assert_eq!(sp.command_id.name(), "Set_Params");

        let gnb = CommsCmd {
            command_id: CommsCommandId::GetNextBuffer,
            params: CommsCmdParams::GetNextBuffer { comms_phase_id: 1 },
        };
        let gb = gnb.to_bytes();
        assert_eq!(gb, [0x9F, 0x8C, 0x00, 0x02, 0x05, 0x01]);
        assert_eq!(CommsCmd::parse(&gb).unwrap(), gnb);
    }

    #[test]
    fn comms_cmd_disconnect_has_no_params() {
        let d = CommsCmd {
            command_id: CommsCommandId::DisconnectOnChannel,
            params: CommsCmdParams::None,
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x00, 0x01, 0x02]);
        assert_eq!(CommsCmd::parse(&bytes).unwrap(), d);
    }

    #[test]
    fn connection_descriptor_standalone_round_trips() {
        let cd = ConnectionDescriptor {
            descriptor_type: ConnectionDescriptorType::SiTelephoneDescriptor,
            payload: &[0xAA, 0xBB, 0xCC],
        };
        let bytes = cd.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x01, 0x04, 0x01, 0xAA, 0xBB, 0xCC]);
        assert_eq!(ConnectionDescriptor::parse(&bytes).unwrap(), cd);
        let mut other = cd.clone();
        other.descriptor_type = ConnectionDescriptorType::CableReturnChannelDescriptor;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn comms_reply_round_trips_and_bites() {
        let r = CommsReply {
            reply_id: CommsReplyId::StatusReply,
            return_value: 0x01, // Connected
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x8C, 0x02, 0x02, 0x04, 0x01]);
        assert_eq!(CommsReply::parse(&bytes).unwrap(), r);
        assert_eq!(r.reply_id.name(), "Status_Reply");
        let mut other = r;
        other.return_value = 0x00;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn comms_send_multibyte_round_trips_and_more_bites() {
        let s = CommsSend {
            more: false,
            comms_phase_id: 0,
            message: b"AT&F\r",
        };
        let bytes = s.to_bytes();
        assert_eq!(
            bytes,
            [0x9F, 0x8C, 0x03, 0x06, 0x00, b'A', b'T', b'&', b'F', b'\r']
        );
        assert_eq!(CommsSend::parse(&bytes).unwrap(), s);

        // more bite: flipping `more` flips the tag.
        let mut more = s.clone();
        more.more = true;
        let mb = more.to_bytes();
        assert_eq!(mb[2], 0x04);
        assert_ne!(bytes, mb);
        assert_eq!(CommsSend::parse(&mb).unwrap(), more);

        // field bite.
        let mut other = s.clone();
        other.comms_phase_id = 1;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn comms_rcv_round_trips_and_more_bites() {
        let r = CommsRcv {
            more: true,
            comms_phase_id: 1,
            message: b"OK\r\n",
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes[2], 0x06); // rcv_more
        assert_eq!(CommsRcv::parse(&bytes).unwrap(), r);

        let mut last = r.clone();
        last.more = false;
        let lb = last.to_bytes();
        assert_eq!(lb[2], 0x05); // rcv_last
        assert_ne!(bytes, lb);
        assert_eq!(CommsRcv::parse(&lb).unwrap(), last);
    }
}
