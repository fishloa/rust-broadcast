//! Power Manager objects — ETSI TS 101 699 V1.1.1 §6.3, Tables 52-55
//! (PDF pp. 51-52). See `docs/ci_plus/power-manager.md`.
//!
//! Resource ID `0x00220041`. Lets a module tell the host it is busy with a task
//! that should complete before the host powers down.
//!
//! - `activation_state_change_request` (`9F 80 00`, Table 52) — host asks the
//!   module to change activation state.
//! - `activation_state_change_ack` (`9F 80 01`, Table 54) — module's reply.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Power Manager (Tables 52, 54).
pub mod tag {
    use crate::tag::ApduTag;
    /// `activation_status_change_request_tag` = `9F 80 00`.
    pub const ACTIVATION_STATE_CHANGE_REQUEST: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `activation_status_change_ack_tag` = `9F 80 01`.
    pub const ACTIVATION_STATE_CHANGE_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
}

/// `activation_state` — the requested power mode (Table 53).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ActivationState {
    /// `0` — Standby-passive (the EACEM-defined power mode).
    StandbyPassive,
    /// `1`-`15` — reserved for future use.
    Reserved(u8),
}

impl ActivationState {
    /// Decode the 4-bit `activation_state`.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::StandbyPassive,
            other => Self::Reserved(other),
        }
    }
    /// 4-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::StandbyPassive => 0,
            Self::Reserved(v) => v & 0x0F,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::StandbyPassive => "Standby-passive",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ActivationState, Reserved);

/// `reply_code` — the module's response to a state-change request (Table 55).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ReplyCode {
    /// `0` — OK to change state.
    Ok,
    /// `1` — Module busy, don't change state.
    Busy,
    /// `2`-`255` — reserved for future use.
    Reserved(u8),
}

impl ReplyCode {
    /// Decode the `reply_code` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Ok,
            1 => Self::Busy,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Busy => 1,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "OK to change state",
            Self::Busy => "Module busy, don't change state",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(ReplyCode, Reserved);

/// `activation_state_change_request()` (Table 52): host → module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ActivationStateChangeRequest {
    /// The requested new activation state (low 4 bits; top 4 bits reserved).
    pub activation_state: ActivationState,
}

/// `activation_state_change_ack()` (Table 54): module → host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ActivationStateChangeAck {
    /// The module's response.
    pub reply_code: ReplyCode,
}

// reserved(4) + activation_state(4).
const REQUEST_BODY: usize = 1;
// reply_code(8).
const ACK_BODY: usize = 1;

impl<'a> Parse<'a> for ActivationStateChangeRequest {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(
            bytes,
            tag::ACTIVATION_STATE_CHANGE_REQUEST,
            "activation_state_change_request",
        )?;
        if body.len() < REQUEST_BODY {
            return Err(Error::BufferTooShort {
                need: REQUEST_BODY,
                have: body.len(),
                what: "activation_state_change_request",
            });
        }
        Ok(Self {
            activation_state: ActivationState::from_u8(body[0] & 0x0F),
        })
    }
}
impl Serialize for ActivationStateChangeRequest {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(REQUEST_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos =
            objects::write_apdu_header(tag::ACTIVATION_STATE_CHANGE_REQUEST, REQUEST_BODY, buf)?;
        // reserved(4)='0000', activation_state(4).
        buf[pos] = self.activation_state.to_u8() & 0x0F;
        Ok(pos + REQUEST_BODY)
    }
}

impl<'a> Parse<'a> for ActivationStateChangeAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(
            bytes,
            tag::ACTIVATION_STATE_CHANGE_ACK,
            "activation_state_change_ack",
        )?;
        if body.len() < ACK_BODY {
            return Err(Error::BufferTooShort {
                need: ACK_BODY,
                have: body.len(),
                what: "activation_state_change_ack",
            });
        }
        Ok(Self {
            reply_code: ReplyCode::from_u8(body[0]),
        })
    }
}
impl Serialize for ActivationStateChangeAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::ACTIVATION_STATE_CHANGE_ACK, ACK_BODY, buf)?;
        buf[pos] = self.reply_code.to_u8();
        Ok(pos + ACK_BODY)
    }
}

/// Resource-scoped dispatch over the Power Manager objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PowerManagerApdu {
    /// `activation_state_change_request` (`9F 80 00`).
    Request(ActivationStateChangeRequest),
    /// `activation_state_change_ack` (`9F 80 01`).
    Ack(ActivationStateChangeAck),
}

impl PowerManagerApdu {
    /// Parse a Power Manager APDU, dispatching on the `apdu_tag`.
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "power_manager apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::ACTIVATION_STATE_CHANGE_REQUEST => {
                Ok(Self::Request(ActivationStateChangeRequest::parse(body)?))
            }
            tag::ACTIVATION_STATE_CHANGE_ACK => {
                Ok(Self::Ack(ActivationStateChangeAck::parse(body)?))
            }
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::ACTIVATION_STATE_CHANGE_REQUEST.as_u24(),
                what: "power_manager",
            }),
        }
    }
}

impl Serialize for PowerManagerApdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::Request(o) => o.serialized_len(),
            Self::Ack(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Request(o) => o.serialize_into(buf),
            Self::Ack(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_and_bites() {
        let r = ActivationStateChangeRequest {
            activation_state: ActivationState::StandbyPassive,
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x01, 0x00]);
        assert_eq!(ActivationStateChangeRequest::parse(&bytes).unwrap(), r);
        assert_eq!(r.activation_state.name(), "Standby-passive");
        let other = ActivationStateChangeRequest {
            activation_state: ActivationState::Reserved(5),
        };
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[4], 0x05);
    }

    #[test]
    fn ack_round_trips_and_bites() {
        let a = ActivationStateChangeAck {
            reply_code: ReplyCode::Busy,
        };
        let bytes = a.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, 0x01]);
        assert_eq!(ActivationStateChangeAck::parse(&bytes).unwrap(), a);
        assert_eq!(a.reply_code.name(), "Module busy, don't change state");
        let other = ActivationStateChangeAck {
            reply_code: ReplyCode::Ok,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn dispatch_routes_both_tags() {
        let req = ActivationStateChangeRequest {
            activation_state: ActivationState::StandbyPassive,
        }
        .to_bytes();
        assert!(matches!(
            PowerManagerApdu::parse(&req).unwrap(),
            PowerManagerApdu::Request(_)
        ));
        let ack = ActivationStateChangeAck {
            reply_code: ReplyCode::Ok,
        }
        .to_bytes();
        let parsed = PowerManagerApdu::parse(&ack).unwrap();
        assert!(matches!(parsed, PowerManagerApdu::Ack(_)));
        assert_eq!(parsed.to_bytes(), ack);
    }
}
