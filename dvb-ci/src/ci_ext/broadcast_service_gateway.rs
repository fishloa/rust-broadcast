//! Broadcast Service Gateway objects — ETSI TS 101 699 V1.1.1 §6.1.3.3,
//! Tables 32-34 (PDF pp. 39-40). See `private/specs-verified/dvb/dvb-ci/ci_plus/input-modules.md`.
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
use crate::reserved::ReservedZero;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::len::need_in;
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
        match v & RESPONSE_CODE_MASK {
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
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

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
    /// The 7-bit `reserved` field in front of `OKToDisruptService` as received, preserved so the
    /// object re-serializes byte-for-byte; `0` when hand-built.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedZero<DISRUPT_FLAG_RESERVED_BITS>,
}

/// One EIT event in an [`EitSectionAck`] loop — modelled on the EIT event loop in
/// ETS 300 468.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EitEvent<'a> {
    /// `event_id` (16-bit).
    pub event_id: u16,
    /// `start_time` — 40-bit MJD+BCD UTC, carried verbatim (5 bytes).
    pub start_time: [u8; START_TIME_LEN],
    /// `duration` — 24-bit BCD HHMMSS.
    pub duration: [u8; DURATION_LEN],
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
    /// How the `length_field` was coded on the wire: a non-minimal long form
    /// (e.g. `82 00 09`) is preserved so the object re-serializes
    /// byte-for-byte; [`LengthForm::Minimal`](crate::length::LengthForm::Minimal) for a hand-built value.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub length_form: crate::length::LengthForm,

    /// `ResponseCode` (Table 34).
    pub response_code: EitResponseCode,
    /// The events (`Length` is the byte count of the event loop, may be 0).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub events: Vec<EitEvent<'a>>,
    /// The 2-bit `reserved` field in front of `ResponseCode` as received, preserved so the
    /// object re-serializes byte-for-byte; `0` when hand-built.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedZero<ACK_RESERVED_BITS>,
}

// --- EITSectionReq ---

// TableID(2) + ServiceID(2) + SectionNumber(1) + OriginalNetworkID(2)
// + Reserved(7)/OKToDisruptService(1).
const EIT_SECTION_REQ_BODY: usize = 2 + 2 + 1 + 2 + 1;

/// Mask of the 2-bit `ResponseCode`.
const RESPONSE_CODE_MASK: u8 = 0x03;
/// Shift placing `ResponseCode` below the reserved bits.
const RESPONSE_CODE_SHIFT: u32 = 4;
/// Shift selecting the reserved bits of the response byte.
const RESPONSE_RESERVED_SHIFT: u32 = 6;
/// Mask of a 12-bit length's high nibble.
const LENGTH_HIGH_MASK: u8 = 0x0F;
/// Shift placing a 12-bit length's high bits.
const LENGTH_SHIFT: u32 = 8;
/// Largest 12-bit length.
const LENGTH_MAX: usize = 0x0FFF;

/// Reserved bits in front of `OKToDisruptService`.
const DISRUPT_FLAG_RESERVED_BITS: u32 = 7;
/// Shift selecting the reserved bits in front of `OKToDisruptService`.
const DISRUPT_FLAG_RESERVED_SHIFT: u32 = 1;
/// Mask of `OKToDisruptService`.
const OK_TO_DISRUPT_MASK: u8 = 0x01;
/// Reserved bits in front of `ResponseCode`.
const ACK_RESERVED_BITS: u32 = 2;

/// Bytes of `start_time` (MJD + BCD HHMMSS).
const START_TIME_LEN: usize = 5;
/// Bytes of `duration` (BCD HHMMSS).
const DURATION_LEN: usize = 3;
/// Offset of `pending_length`'s low byte in an `EITSectionAck`.
const ACK_LENGTH_LOW_OFFSET: usize = 1;

/// Offsets within an `EITSectionReq` body.
const TABLE_ID_OFFSET: usize = 0;
const SERVICE_ID_OFFSET: usize = 2;
const SECTION_NUMBER_OFFSET: usize = 4;
const ORIGINAL_NETWORK_ID_OFFSET: usize = 5;
const DISRUPT_FLAGS_OFFSET: usize = 7;

/// Offsets within an EIT event entry.
const EVENT_ID_OFFSET: usize = 0;
const START_TIME_OFFSET: usize = 2;
const DURATION_OFFSET: usize = 7;
const EVENT_FLAGS_OFFSET: usize = 10;
const LOOP_LENGTH_LOW_OFFSET: usize = 11;
/// Shift placing `running_status` at the top of the event flags byte.
const RUNNING_STATUS_SHIFT: u32 = 5;
/// Mask of `running_status`.
const RUNNING_STATUS_MASK: u8 = 0x07;
/// Bit width of `running_status`.
const RUNNING_STATUS_BITS: u32 = 3;
/// Mask of `free_CA_mode`.
const FREE_CA_MODE_MASK: u8 = 0x10;
/// Shift placing `free_CA_mode`.
const FREE_CA_MODE_SHIFT: u32 = 4;

impl<'a> Parse<'a> for EitSectionReq {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (length_form, body) =
            objects::parse_apdu_header(bytes, tag::EIT_SECTION_REQ, "EITSectionReq")?;
        need_in(body, EIT_SECTION_REQ_BODY, "EITSectionReq")?;
        crate::objects::reject_trailing_body(body, EIT_SECTION_REQ_BODY, "EITSectionReq")?;
        Ok(Self {
            length_form,
            table_id: u16::from_be_bytes([body[TABLE_ID_OFFSET], body[TABLE_ID_OFFSET + 1]]),
            service_id: u16::from_be_bytes([body[SERVICE_ID_OFFSET], body[SERVICE_ID_OFFSET + 1]]),
            section_number: body[SECTION_NUMBER_OFFSET],
            original_network_id: u16::from_be_bytes([
                body[ORIGINAL_NETWORK_ID_OFFSET],
                body[ORIGINAL_NETWORK_ID_OFFSET + 1],
            ]),
            ok_to_disrupt_service: (body[DISRUPT_FLAGS_OFFSET] & OK_TO_DISRUPT_MASK) != 0,
            reserved: ReservedZero::from_bits(u64::from(
                body[DISRUPT_FLAGS_OFFSET] >> DISRUPT_FLAG_RESERVED_SHIFT,
            )),
        })
    }
}
impl Serialize for EitSectionReq {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(EIT_SECTION_REQ_BODY, self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(
            tag::EIT_SECTION_REQ,
            EIT_SECTION_REQ_BODY,
            self.length_form,
            buf,
        )?;
        buf[pos + TABLE_ID_OFFSET..pos + SERVICE_ID_OFFSET]
            .copy_from_slice(&self.table_id.to_be_bytes());
        buf[pos + SERVICE_ID_OFFSET..pos + SECTION_NUMBER_OFFSET]
            .copy_from_slice(&self.service_id.to_be_bytes());
        buf[pos + SECTION_NUMBER_OFFSET] = self.section_number;
        buf[pos + ORIGINAL_NETWORK_ID_OFFSET..pos + DISRUPT_FLAGS_OFFSET]
            .copy_from_slice(&self.original_network_id.to_be_bytes());
        // Reserved(7), OKToDisruptService(1).
        buf[pos + DISRUPT_FLAGS_OFFSET] = ((self.reserved.get() << DISRUPT_FLAG_RESERVED_SHIFT)
            as u8)
            | u8::from(self.ok_to_disrupt_service);
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
        let (length_form, body) =
            objects::parse_apdu_header(bytes, tag::EIT_SECTION_ACK, "EITSectionAck")?;
        need_in(body, EIT_SECTION_ACK_PREFIX, "EITSectionAck")?;
        // byte0: Reserved(2) + ResponseCode(2) + Length high nibble(4 → top of 12).
        let response_code =
            EitResponseCode::from_u8((body[0] >> RESPONSE_CODE_SHIFT) & RESPONSE_CODE_MASK);
        let length = ((u16::from(body[0] & LENGTH_HIGH_MASK) << LENGTH_SHIFT)
            | u16::from(body[ACK_LENGTH_LOW_OFFSET])) as usize;
        let loop_start = EIT_SECTION_ACK_PREFIX;
        let loop_end = loop_start + length;
        if body.len() < loop_end {
            return Err(Error::LengthMismatch(
                broadcast_common::error::LengthMismatch {
                    what: "EITSectionAck event loop",
                    declared: length,
                    actual: body.len() - loop_start,
                },
            ));
        }
        objects::reject_trailing_body(body, loop_end, "EITSectionAck")?;
        let mut events = Vec::new();
        let mut p = loop_start;
        while p < loop_end {
            if loop_end - p < EIT_EVENT_FIXED {
                return Err(Error::InvalidObject {
                    what: "EITSectionAck event",
                    reason: "truncated event header",
                });
            }
            let event_id =
                u16::from_be_bytes([body[p + EVENT_ID_OFFSET], body[p + EVENT_ID_OFFSET + 1]]);
            let mut start_time = [0u8; START_TIME_LEN];
            start_time.copy_from_slice(
                &body[p + START_TIME_OFFSET..p + START_TIME_OFFSET + START_TIME_LEN],
            );
            let mut duration = [0u8; DURATION_LEN];
            duration
                .copy_from_slice(&body[p + DURATION_OFFSET..p + DURATION_OFFSET + DURATION_LEN]);
            let b10 = body[p + EVENT_FLAGS_OFFSET];
            let b11 = body[p + LOOP_LENGTH_LOW_OFFSET];
            let running_status = (b10 >> RUNNING_STATUS_SHIFT) & RUNNING_STATUS_MASK;
            let free_ca_mode = (b10 & FREE_CA_MODE_MASK) != 0;
            let dll =
                ((u16::from(b10 & LENGTH_HIGH_MASK) << LENGTH_SHIFT) | u16::from(b11)) as usize;
            let desc_start = p + EIT_EVENT_FIXED;
            let desc_end = desc_start + dll;
            if desc_end > loop_end {
                return Err(Error::LengthMismatch(
                    broadcast_common::error::LengthMismatch {
                        what: "EITSectionAck event descriptors",
                        declared: dll,
                        actual: loop_end - desc_start,
                    },
                ));
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
            length_form,
            response_code,
            events,
            reserved: ReservedZero::from_bits(u64::from(body[0] >> RESPONSE_RESERVED_SHIFT)),
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
        objects::apdu_len(EIT_SECTION_ACK_PREFIX + self.loop_len(), self.length_form)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let length = self.loop_len();
        if length > LENGTH_MAX {
            return Err(Error::InvalidObject {
                what: "EITSectionAck",
                reason: "event loop longer than 4095 bytes",
            });
        }
        let body_len = EIT_SECTION_ACK_PREFIX + length;
        let mut pos =
            objects::write_apdu_header(tag::EIT_SECTION_ACK, body_len, self.length_form, buf)?;
        let len16 = length as u16;
        // Reserved(2), ResponseCode(2), Length(12).
        buf[pos] = ((self.reserved.get() << RESPONSE_RESERVED_SHIFT) as u8)
            | (self.response_code.to_u8() << RESPONSE_CODE_SHIFT)
            | ((len16 >> LENGTH_SHIFT) as u8 & LENGTH_HIGH_MASK);
        buf[pos + 1] = len16 as u8;
        pos += EIT_SECTION_ACK_PREFIX;
        for e in &self.events {
            if e.descriptors.len() > LENGTH_MAX {
                return Err(Error::InvalidObject {
                    what: "EITSectionAck event",
                    reason: "event descriptors loop longer than 4095 bytes",
                });
            }
            objects::fit_bits(
                u64::from(e.running_status),
                RUNNING_STATUS_BITS,
                "EITSectionAck event running_status",
            )?;
            buf[pos + EVENT_ID_OFFSET..pos + START_TIME_OFFSET]
                .copy_from_slice(&e.event_id.to_be_bytes());
            buf[pos + START_TIME_OFFSET..pos + DURATION_OFFSET].copy_from_slice(&e.start_time);
            buf[pos + DURATION_OFFSET..pos + EVENT_FLAGS_OFFSET].copy_from_slice(&e.duration);
            let dll = e.descriptors.len() as u16;
            // running_status range-checked above.
            buf[pos + EVENT_FLAGS_OFFSET] = (e.running_status << RUNNING_STATUS_SHIFT)
                | (u8::from(e.free_ca_mode) << FREE_CA_MODE_SHIFT)
                | ((dll >> LENGTH_SHIFT) as u8 & LENGTH_HIGH_MASK);
            buf[pos + LOOP_LENGTH_LOW_OFFSET] = dll as u8;
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
        need_in(
            body,
            crate::objects::APDU_TAG_LEN,
            "broadcast_service_gateway apdu_tag",
        )?;
        let t = ApduTag::from_bytes(body[0], body[1], body[crate::objects::APDU_TAG_LEN - 1]);
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
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            table_id: 0x004E,
            service_id: 0x0064,
            section_number: 0x00,
            original_network_id: 0x0001,
            ok_to_disrupt_service: true,
        };
        let bytes = req.try_to_bytes().unwrap();
        // body 8; tag(3)+len(1)+8 = 12; byte7 = 0x01 (OKToDisrupt).
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x10, 0x08, 0x00, 0x4E, 0x00, 0x64, 0x00, 0x00, 0x01, 0x01
            ]
        );
        assert_eq!(EitSectionReq::parse(&bytes).unwrap(), req);
        let mut other = req;
        other.ok_to_disrupt_service = false;
        assert_ne!(bytes, other.try_to_bytes().unwrap());
        assert_eq!(other.try_to_bytes().unwrap()[11], 0x00);
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
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            response_code: EitResponseCode::SectionFound,
            events: alloc::vec![e0, e1],
        };
        let bytes = ack.try_to_bytes().unwrap();
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
        assert_ne!(bytes, other.try_to_bytes().unwrap());
    }

    #[test]
    fn eit_section_ack_empty_loop() {
        let ack = EitSectionAck {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            response_code: EitResponseCode::NotAvailable,
            events: alloc::vec![],
        };
        let bytes = ack.try_to_bytes().unwrap();
        // byte4 = (0b01<<4)|0 = 0x10 ; Length = 0.
        assert_eq!(bytes, [0x9F, 0x80, 0x11, 0x02, 0x10, 0x00]);
        assert_eq!(EitSectionAck::parse(&bytes).unwrap(), ack);
        assert_eq!(ack.response_code.name(), "Section not available");
    }

    #[test]
    fn bsg_routes_eit_tags() {
        let req = EitSectionReq {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            table_id: 0x4E,
            service_id: 1,
            section_number: 0,
            original_network_id: 1,
            ok_to_disrupt_service: false,
        }
        .try_to_bytes()
        .unwrap();
        assert!(matches!(
            BroadcastServiceGatewayApdu::parse(&req).unwrap(),
            BroadcastServiceGatewayApdu::EitSectionReq(_)
        ));
    }

    #[test]
    fn bsg_delegates_generic_gateway_tags() {
        // 9F8000 (ServiceListReq) is inherited from the Generic Service Gateway.
        let req = ServiceListReq::default().try_to_bytes().unwrap();
        let parsed = BroadcastServiceGatewayApdu::parse(&req).unwrap();
        assert!(matches!(
            parsed,
            BroadcastServiceGatewayApdu::ServiceGateway(ServiceGatewayApdu::ServiceListReq(_))
        ));
        assert_eq!(parsed.try_to_bytes().unwrap(), req);

        // 9F8006 ServiceDescAck inherited too.
        use super::super::service_gateway::ServiceDescAck;
        let sda = ServiceDescAck {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
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
        .try_to_bytes()
        .unwrap();
        let parsed = BroadcastServiceGatewayApdu::parse(&sda).unwrap();
        assert!(matches!(
            parsed,
            BroadcastServiceGatewayApdu::ServiceGateway(ServiceGatewayApdu::ServiceDescAck(_))
        ));
        assert_eq!(parsed.try_to_bytes().unwrap(), sda);
    }

    #[test]
    fn oversized_event_running_status_is_rejected_not_wrapped() {
        // Before the fix, 0x0A (exceeds 3 bits) silently wrapped to 0x02 in
        // the event's running_status field and returned Ok.
        let e = EitEvent {
            event_id: 1,
            start_time: [0; 5],
            duration: [0; 3],
            running_status: 0x0A,
            free_ca_mode: false,
            descriptors: &[],
        };
        let ack = EitSectionAck {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            response_code: EitResponseCode::SectionFound,
            events: alloc::vec![e],
        };
        let mut buf = [0u8; 64];
        assert!(matches!(
            ack.serialize_into(&mut buf),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn max_event_running_status_still_serializes_and_round_trips() {
        let e = EitEvent {
            event_id: 1,
            start_time: [0; 5],
            duration: [0; 3],
            running_status: 0x07, // the 3-bit maximum
            free_ca_mode: false,
            descriptors: &[],
        };
        let ack = EitSectionAck {
            reserved: Default::default(),
            length_form: crate::length::LengthForm::Minimal,
            response_code: EitResponseCode::SectionFound,
            events: alloc::vec![e],
        };
        let bytes = ack.try_to_bytes().unwrap();
        assert_eq!(EitSectionAck::parse(&bytes).unwrap(), ack);
    }
}
