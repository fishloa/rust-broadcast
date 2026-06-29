//! Broadcast Service Gateway objects — ETSI TS 101 699 V1.1.1 §6.1.3.3,
//! Tables 32-34 (PDF pp. 39-40). See `docs/ci_plus/input-modules.md`.
//!
//! Resource ID `0x00811ii1` (`ii` = Module ID). A Type 'B' module on a broadcast
//! network presents this resource. It **inherits all Generic Service Gateway
//! calls** (Tables 22-31, tags `9F8000`-`9F8008` — see
//! [`super::service_gateway`]) and adds the broadcast-event (EIT) extension
//! objects:
//!
//! - `EITSectionReq` (`9F 80 10`, Table 32) — app → module: request an EIT section.
//! - `EITSectionAck` (`9F 80 11`, Table 33) — module → app: response code +
//!   EIT-modelled event loop.
//!
//! Dispatch ([`BroadcastServiceGatewayApdu`]) routes `9F8010`/`9F8011` to the EIT
//! objects and **delegates every other `9F80xx` tag to the inherited Generic
//! Service Gateway dispatch**.

use super::service_gateway::ServiceGatewayApdu;
use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Broadcast Service Gateway EIT extension
/// (Tables 32-33). The generic-gateway tags live in [`super::service_gateway::tag`].
pub mod tag {
    use crate::tag::ApduTag;
    /// `EITSectionReqTag` = `9F 80 10`.
    pub const EIT_SECTION_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x10);
    /// `EITSectionAckTag` = `9F 80 11`.
    pub const EIT_SECTION_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x11);
}

/// `ResponseCode` — EIT section response status (Table 34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EitResponseCode {
    /// `0b00` — Section not on the present document (may be on another TS).
    NotOnPresentDocument,
    /// `0b01` — Section not available.
    NotAvailable,
    /// `0b10` — Section found.
    SectionFound,
    /// `0b11` — reserved.
    Reserved,
}

impl EitResponseCode {
    /// Decode the 2-bit `ResponseCode`.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0b00 => Self::NotOnPresentDocument,
            0b01 => Self::NotAvailable,
            0b10 => Self::SectionFound,
            _ => Self::Reserved,
        }
    }
    /// 2-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NotOnPresentDocument => 0b00,
            Self::NotAvailable => 0b01,
            Self::SectionFound => 0b10,
            Self::Reserved => 0b11,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotOnPresentDocument => "Section not on the present document",
            Self::NotAvailable => "Section not available",
            Self::SectionFound => "Section found",
            Self::Reserved => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(EitResponseCode);

/// `EITSectionReq()` (Table 32) — app → module: request an EIT section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EitSectionReq {
    /// `TableID` — 16-bit (wider than the 8-bit EIT table_id); EIT values
    /// `0x4E`-`0x6F` (ETS 300 468).
    pub table_id: u16,
    /// `ServiceID` — as the EIT in ETS 300 468.
    pub service_id: u16,
    /// `SectionNumber` — as the EIT in ETS 300 468.
    pub section_number: u8,
    /// `OriginalNetworkID` — as the EIT in ETS 300 468.
    pub original_network_id: u16,
    /// `OKToDisruptService` — `1`: a current service may be disrupted to obtain
    /// the requested event information; `0`: delivery shall not be disrupted.
    pub ok_to_disrupt_service: bool,
}

/// One EIT event in an [`EitSectionAck`] loop — modelled on the EIT event loop in
/// ETS 300 468.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EitEvent<'a> {
    /// `event_id` (16-bit).
    pub event_id: u16,
    /// `start_time` — 40-bit MJD+BCD UTC, carried verbatim (5 bytes).
    pub start_time: [u8; 5],
    /// `duration` — 24-bit BCD HHMMSS.
    pub duration: [u8; 3],
    /// `running_status` — 3-bit EIT running status.
    pub running_status: u8,
    /// `free_CA_mode` — EIT meaning.
    pub free_ca_mode: bool,
    /// The EIT event descriptor loop (`descriptors_loop_length` bytes), verbatim.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub descriptors: &'a [u8],
}

/// `EITSectionAck()` (Table 33) — module → app: response code + an EIT event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EitSectionAck<'a> {
    /// `ResponseCode` (Table 34).
    pub response_code: EitResponseCode,
    /// The events (`Length` is the byte count of the event loop, may be 0).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub events: Vec<EitEvent<'a>>,
}

// --- EITSectionReq ---

// TableID(2) + ServiceID(2) + SectionNumber(1) + OriginalNetworkID(2)
// + Reserved(7)/OKToDisruptService(1).
const EIT_SECTION_REQ_BODY: usize = 2 + 2 + 1 + 2 + 1;

impl<'a> Parse<'a> for EitSectionReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::EIT_SECTION_REQ, "EITSectionReq")?;
        if body.len() < EIT_SECTION_REQ_BODY {
            return Err(Error::BufferTooShort {
                need: EIT_SECTION_REQ_BODY,
                have: body.len(),
                what: "EITSectionReq",
            });
        }
        Ok(Self {
            table_id: u16::from_be_bytes([body[0], body[1]]),
            service_id: u16::from_be_bytes([body[2], body[3]]),
            section_number: body[4],
            original_network_id: u16::from_be_bytes([body[5], body[6]]),
            ok_to_disrupt_service: (body[7] & 0x01) != 0,
        })
    }
}
impl Serialize for EitSectionReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EIT_SECTION_REQ_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::EIT_SECTION_REQ, EIT_SECTION_REQ_BODY, buf)?;
        buf[pos..pos + 2].copy_from_slice(&self.table_id.to_be_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&self.service_id.to_be_bytes());
        buf[pos + 4] = self.section_number;
        buf[pos + 5..pos + 7].copy_from_slice(&self.original_network_id.to_be_bytes());
        // Reserved(7)=0, OKToDisruptService(1).
        buf[pos + 7] = u8::from(self.ok_to_disrupt_service);
        Ok(pos + EIT_SECTION_REQ_BODY)
    }
}

// --- EITSectionAck ---

// Reserved(2)/ResponseCode(2)/Length(12) = 2 header bytes.
const EIT_SECTION_ACK_PREFIX: usize = 2;
// event_id(2) + start_time(5) + duration(3) + running_status/free_CA/loop_len(2).
const EIT_EVENT_FIXED: usize = 2 + 5 + 3 + 2;

impl<'a> Parse<'a> for EitSectionAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::EIT_SECTION_ACK, "EITSectionAck")?;
        if body.len() < EIT_SECTION_ACK_PREFIX {
            return Err(Error::BufferTooShort {
                need: EIT_SECTION_ACK_PREFIX,
                have: body.len(),
                what: "EITSectionAck",
            });
        }
        // byte0: Reserved(2) + ResponseCode(2) + Length high nibble(4 → top of 12).
        let response_code = EitResponseCode::from_u8((body[0] >> 4) & 0x03);
        let length = ((u16::from(body[0] & 0x0F) << 8) | u16::from(body[1])) as usize;
        let loop_start = EIT_SECTION_ACK_PREFIX;
        let loop_end = loop_start + length;
        if body.len() < loop_end {
            return Err(Error::LengthMismatch {
                what: "EITSectionAck event loop",
                declared: length,
                actual: body.len() - loop_start,
            });
        }
        let mut events = Vec::new();
        let mut p = loop_start;
        while p < loop_end {
            if loop_end - p < EIT_EVENT_FIXED {
                return Err(Error::InvalidObject {
                    what: "EITSectionAck event",
                    reason: "truncated event header",
                });
            }
            let event_id = u16::from_be_bytes([body[p], body[p + 1]]);
            let mut start_time = [0u8; 5];
            start_time.copy_from_slice(&body[p + 2..p + 7]);
            let mut duration = [0u8; 3];
            duration.copy_from_slice(&body[p + 7..p + 10]);
            let b10 = body[p + 10];
            let b11 = body[p + 11];
            let running_status = (b10 >> 5) & 0x07;
            let free_ca_mode = (b10 & 0x10) != 0;
            let dll = ((u16::from(b10 & 0x0F) << 8) | u16::from(b11)) as usize;
            let desc_start = p + EIT_EVENT_FIXED;
            let desc_end = desc_start + dll;
            if desc_end > loop_end {
                return Err(Error::LengthMismatch {
                    what: "EITSectionAck event descriptors",
                    declared: dll,
                    actual: loop_end - desc_start,
                });
            }
            events.push(EitEvent {
                event_id,
                start_time,
                duration,
                running_status,
                free_ca_mode,
                descriptors: &body[desc_start..desc_end],
            });
            p = desc_end;
        }
        Ok(Self {
            response_code,
            events,
        })
    }
}

impl EitSectionAck<'_> {
    /// Byte length of the event loop (`Length`).
    fn loop_len(&self) -> usize {
        self.events
            .iter()
            .map(|e| EIT_EVENT_FIXED + e.descriptors.len())
            .sum()
    }
}

impl Serialize for EitSectionAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EIT_SECTION_ACK_PREFIX + self.loop_len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let length = self.loop_len();
        if length > 0x0FFF {
            return Err(Error::InvalidObject {
                what: "EITSectionAck",
                reason: "event loop longer than 4095 bytes",
            });
        }
        let body_len = EIT_SECTION_ACK_PREFIX + length;
        let mut pos = objects::write_apdu_header(tag::EIT_SECTION_ACK, body_len, buf)?;
        let len16 = length as u16;
        // Reserved(2)=0, ResponseCode(2), Length(12).
        buf[pos] = (self.response_code.to_u8() << 4) | ((len16 >> 8) as u8 & 0x0F);
        buf[pos + 1] = len16 as u8;
        pos += EIT_SECTION_ACK_PREFIX;
        for e in &self.events {
            if e.descriptors.len() > 0x0FFF {
                return Err(Error::InvalidObject {
                    what: "EITSectionAck event",
                    reason: "event descriptors loop longer than 4095 bytes",
                });
            }
            buf[pos..pos + 2].copy_from_slice(&e.event_id.to_be_bytes());
            buf[pos + 2..pos + 7].copy_from_slice(&e.start_time);
            buf[pos + 7..pos + 10].copy_from_slice(&e.duration);
            let dll = e.descriptors.len() as u16;
            buf[pos + 10] = ((e.running_status & 0x07) << 5)
                | (u8::from(e.free_ca_mode) << 4)
                | ((dll >> 8) as u8 & 0x0F);
            buf[pos + 11] = dll as u8;
            pos += EIT_EVENT_FIXED;
            buf[pos..pos + e.descriptors.len()].copy_from_slice(e.descriptors);
            pos += e.descriptors.len();
        }
        Ok(pos)
    }
}

/// Resource-scoped dispatch over the Broadcast Service Gateway objects.
///
/// `9F8010`/`9F8011` route to the EIT extension objects; every other `9F80xx` tag
/// is delegated to the inherited [`ServiceGatewayApdu`] (Tables 22-30).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BroadcastServiceGatewayApdu<'a> {
    /// An inherited Generic Service Gateway object (`9F8000`-`9F8008`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    ServiceGateway(ServiceGatewayApdu<'a>),
    /// `EITSectionReq` (`9F 80 10`).
    EitSectionReq(EitSectionReq),
    /// `EITSectionAck` (`9F 80 11`).
    EitSectionAck(EitSectionAck<'a>),
}

impl<'a> BroadcastServiceGatewayApdu<'a> {
    /// Parse a Broadcast Service Gateway APDU. `9F8010`/`9F8011` route to the EIT
    /// objects; any other tag is delegated to the inherited Generic Service
    /// Gateway dispatch.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "broadcast_service_gateway apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::EIT_SECTION_REQ => Ok(Self::EitSectionReq(EitSectionReq::parse(body)?)),
            tag::EIT_SECTION_ACK => Ok(Self::EitSectionAck(EitSectionAck::parse(body)?)),
            // All other 9F80xx tags are the inherited Generic Service Gateway calls.
            _ => Ok(Self::ServiceGateway(ServiceGatewayApdu::parse(body)?)),
        }
    }
}

impl Serialize for BroadcastServiceGatewayApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::ServiceGateway(o) => o.serialized_len(),
            Self::EitSectionReq(o) => o.serialized_len(),
            Self::EitSectionAck(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::ServiceGateway(o) => o.serialize_into(buf),
            Self::EitSectionReq(o) => o.serialize_into(buf),
            Self::EitSectionAck(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::service_gateway::{ServiceListReq, ServiceReference};
    use super::*;

    #[test]
    fn eit_section_req_round_trips_and_bites() {
        let req = EitSectionReq {
            table_id: 0x004E,
            service_id: 0x0064,
            section_number: 0x00,
            original_network_id: 0x0001,
            ok_to_disrupt_service: true,
        };
        let bytes = req.to_bytes();
        // body 8; tag(3)+len(1)+8 = 12; byte7 = 0x01 (OKToDisrupt).
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x10, 0x08, 0x00, 0x4E, 0x00, 0x64, 0x00, 0x00, 0x01, 0x01]
        );
        assert_eq!(EitSectionReq::parse(&bytes).unwrap(), req);
        let mut other = req;
        other.ok_to_disrupt_service = false;
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[11], 0x00);
    }

    #[test]
    fn eit_section_ack_multi_event_round_trips_and_bites() {
        // Two events; first carries a 2-byte descriptor, second is empty.
        let d0 = [0x4D, 0x00]; // short_event_descriptor tag, empty-ish
        let e0 = EitEvent {
            event_id: 0x1001,
            start_time: [0xC0, 0x79, 0x12, 0x45, 0x00],
            duration: [0x01, 0x30, 0x00],
            running_status: 4,
            free_ca_mode: false,
            descriptors: &d0,
        };
        let e1 = EitEvent {
            event_id: 0x1002,
            start_time: [0xC0, 0x79, 0x14, 0x15, 0x00],
            duration: [0x00, 0x45, 0x00],
            running_status: 1,
            free_ca_mode: true,
            descriptors: &[],
        };
        let ack = EitSectionAck {
            response_code: EitResponseCode::SectionFound,
            events: alloc::vec![e0, e1],
        };
        let bytes = ack.to_bytes();
        // event0 = 12 fixed + 2 desc = 14; event1 = 12; Length = 26 = 0x1A.
        // byte4(prefix0) = (0b10<<4) | (26>>8) = 0x20 ; byte5 = 26 = 0x1A.
        assert_eq!(bytes[0..6], [0x9F, 0x80, 0x11, 0x1C, 0x20, 0x1A]);
        // event0 running_status=4, free_CA=0, dll=2 → byte = (4<<5)|0|0 = 0x80, next 0x02
        assert_eq!(bytes[16], 0x80);
        assert_eq!(bytes[17], 0x02);
        let parsed = EitSectionAck::parse(&bytes).unwrap();
        assert_eq!(parsed, ack);
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.events[0].running_status, 4);
        assert!(parsed.events[1].free_ca_mode);
        let mut other = ack.clone();
        other.events[0].event_id = 0x1003;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn eit_section_ack_empty_loop() {
        let ack = EitSectionAck {
            response_code: EitResponseCode::NotAvailable,
            events: alloc::vec![],
        };
        let bytes = ack.to_bytes();
        // byte4 = (0b01<<4)|0 = 0x10 ; Length = 0.
        assert_eq!(bytes, [0x9F, 0x80, 0x11, 0x02, 0x10, 0x00]);
        assert_eq!(EitSectionAck::parse(&bytes).unwrap(), ack);
        assert_eq!(ack.response_code.name(), "Section not available");
    }

    #[test]
    fn bsg_routes_eit_tags() {
        let req = EitSectionReq {
            table_id: 0x4E,
            service_id: 1,
            section_number: 0,
            original_network_id: 1,
            ok_to_disrupt_service: false,
        }
        .to_bytes();
        assert!(matches!(
            BroadcastServiceGatewayApdu::parse(&req).unwrap(),
            BroadcastServiceGatewayApdu::EitSectionReq(_)
        ));
    }

    #[test]
    fn bsg_delegates_generic_gateway_tags() {
        // 9F8000 (ServiceListReq) is inherited from the Generic Service Gateway.
        let req = ServiceListReq.to_bytes();
        let parsed = BroadcastServiceGatewayApdu::parse(&req).unwrap();
        assert!(matches!(
            parsed,
            BroadcastServiceGatewayApdu::ServiceGateway(ServiceGatewayApdu::ServiceListReq(_))
        ));
        assert_eq!(parsed.to_bytes(), req);

        // 9F8006 ServiceDescAck inherited too.
        use super::super::service_gateway::ServiceDescAck;
        let sda = ServiceDescAck {
            service: ServiceReference {
                original_network_id: 1,
                service_id: 0x64,
            },
            eit_schedule_flag: true,
            eit_present_following_flag: true,
            running_status: 4,
            free_ca_mode: false,
            descriptors: &[0x48, 0x01, 0x01],
        }
        .to_bytes();
        let parsed = BroadcastServiceGatewayApdu::parse(&sda).unwrap();
        assert!(matches!(
            parsed,
            BroadcastServiceGatewayApdu::ServiceGateway(ServiceGatewayApdu::ServiceDescAck(_))
        ));
        assert_eq!(parsed.to_bytes(), sda);
    }
}
