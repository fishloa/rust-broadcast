//! StreamInput objects — ETSI TS 101 699 V1.1.1 §6.1.2, Tables 12-20
//! (PDF pp. 25-28). See `docs/ci_plus/input-modules.md`.
//!
//! Resource ID `0x00801ii1` (`ii` = Module ID), single session. A Type 'A' input
//! module presents StreamInput: it delivers broadcast services at the **TS
//! level**; the host scans for, and tunes to, transport streams.
//!
//! - `DeliverySystemInfoReq` (`9F 80 00`, Table 13) — host → module: header-only.
//! - `DeliverySystemInfoAck` (`9F 80 01`, Table 14) — module → host: a list of
//!   `SystemIdentifier`s (Table 15: 0=Abstract, 1=DVB-C, 2=DVB-S, 3=DVB-T).
//! - `ScanStartReq` (`9F 80 02`, Table 16) — host → module: header-only.
//! - `ScanNextReq` (`9F 80 03`, Table 17) — host → module: header-only.
//! - `ScanAck` (`9F 80 04`, Table 18) — module → host: `TSState` + 11-byte
//!   `TuningInformationMessage` + `ScanProgress`.
//! - `TuneTSReq` (`9F 80 05`, Table 19) — host → module: an (optional) 11-byte
//!   `TuningInformationMessage` (absent = disconnect from network).
//! - `TuneTSAck` (`9F 80 06`, Table 20) — module → host: `TSState`.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for StreamInput (Tables 13-20).
pub mod tag {
    use crate::tag::ApduTag;
    /// `DeliverySystemInfoReqTag` = `9F 80 00`.
    pub const DELIVERY_SYSTEM_INFO_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `DeliverySystemInfoAckTag` = `9F 80 01`.
    pub const DELIVERY_SYSTEM_INFO_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `ScanStartReqTag` = `9F 80 02`.
    pub const SCAN_START_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `ScanNextReqTag` = `9F 80 03`.
    pub const SCAN_NEXT_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
    /// `ScanAckTag` = `9F 80 04`.
    pub const SCAN_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x04);
    /// `TuneTSReqTag` = `9F 80 05`.
    pub const TUNE_TS_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x05);
    /// `TuneTSAckTag` = `9F 80 06`.
    pub const TUNE_TS_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x06);
}

/// Length of a `TuningInformationMessage` — always 11 bytes (`11 x 8`, §6.1.1).
pub const TUNING_INFO_MESSAGE_LEN: usize = 11;

/// `SystemIdentifier` — the delivery system a module connects to (Table 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SystemIdentifier {
    /// `0` — "Abstract" (tuning info is module-specific).
    Abstract,
    /// `1` — DVB-C (tuning info as the DVB SI cable delivery system descriptor).
    DvbC,
    /// `2` — DVB-S (tuning info as the DVB SI satellite delivery system descriptor).
    DvbS,
    /// `3` — DVB-T (tuning info as the DVB SI terrestrial delivery system descriptor).
    DvbT,
    /// `> 3` — reserved for future use.
    Reserved(u8),
}

impl SystemIdentifier {
    /// Decode a `SystemIdentifier` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Abstract,
            1 => Self::DvbC,
            2 => Self::DvbS,
            3 => Self::DvbT,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Abstract => 0,
            Self::DvbC => 1,
            Self::DvbS => 2,
            Self::DvbT => 3,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Abstract => "Abstract",
            Self::DvbC => "DVB-C",
            Self::DvbS => "DVB-S",
            Self::DvbT => "DVB-T",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(SystemIdentifier, Reserved);

/// `DeliverySystemInfoReq()` (Table 13) — host → module: header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DeliverySystemInfoReq;

/// `ScanStartReq()` (Table 16) — host → module: header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScanStartReq;

/// `ScanNextReq()` (Table 17) — host → module: header-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScanNextReq;

/// `DeliverySystemInfoAck()` (Table 14) — module → host: the delivery systems the
/// module is connected to (one `SystemIdentifier` per byte).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DeliverySystemInfoAck {
    /// `SystemIdentifier`s in wire order (`length_field = N`).
    pub systems: Vec<SystemIdentifier>,
}

/// `ScanAck()` (Table 18) — module → host: a TS found during a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScanAck<'a> {
    /// `TSState` — `0` = no signal (or, when auto-scanning, all frequencies
    /// searched); `1`-`255` = normalized signal-quality (bigger is better).
    pub ts_state: u8,
    /// `TuningInformationMessage` — 11-byte delivery-system-dependent coding to
    /// re-acquire the TS. Undefined when `ts_state == 0`.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub tuning_information_message: &'a [u8],
    /// `ScanProgress` — 0-255, approximate proportional indication of scan progress.
    pub scan_progress: u8,
}

/// `TuneTSReq()` (Table 19) — host → module: tune to a TS. An absent
/// `TuningInformationMessage` (zero following bytes) requests a network disconnect.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TuneTSReq<'a> {
    /// `TuningInformationMessage` — 11 bytes, or empty (= disconnect from network).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub tuning_information_message: &'a [u8],
}

/// `TuneTSAck()` (Table 20) — module → host: tune result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TuneTSAck {
    /// `TSState` — identical coding to the [`ScanAck`] `TSState`; `0` after a
    /// disconnect request.
    pub ts_state: u8,
}

// --- header-only objects ---

macro_rules! empty_object {
    ($ty:ty, $tag:expr, $what:literal) => {
        impl<'a> Parse<'a> for $ty {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                objects::parse_empty_apdu(bytes, $tag, $what)?;
                Ok(Self)
            }
        }
        impl Serialize for $ty {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::empty_apdu_len()
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                objects::serialize_empty_apdu($tag, buf)
            }
        }
    };
}

empty_object!(
    DeliverySystemInfoReq,
    tag::DELIVERY_SYSTEM_INFO_REQ,
    "DeliverySystemInfoReq"
);
empty_object!(ScanStartReq, tag::SCAN_START_REQ, "ScanStartReq");
empty_object!(ScanNextReq, tag::SCAN_NEXT_REQ, "ScanNextReq");

// --- DeliverySystemInfoAck ---

impl<'a> Parse<'a> for DeliverySystemInfoAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(
            bytes,
            tag::DELIVERY_SYSTEM_INFO_ACK,
            "DeliverySystemInfoAck",
        )?;
        let mut systems = Vec::with_capacity(body.len());
        for &b in body {
            systems.push(SystemIdentifier::from_u8(b));
        }
        Ok(Self { systems })
    }
}
impl Serialize for DeliverySystemInfoAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.systems.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.systems.len();
        let mut pos = objects::write_apdu_header(tag::DELIVERY_SYSTEM_INFO_ACK, body_len, buf)?;
        for s in &self.systems {
            buf[pos] = s.to_u8();
            pos += 1;
        }
        Ok(pos)
    }
}

// --- ScanAck ---

// TSState(1) + TuningInformationMessage(11) + ScanProgress(1).
const SCAN_ACK_BODY: usize = 1 + TUNING_INFO_MESSAGE_LEN + 1;

impl<'a> Parse<'a> for ScanAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::SCAN_ACK, "ScanAck")?;
        if body.len() < SCAN_ACK_BODY {
            return Err(Error::BufferTooShort {
                need: SCAN_ACK_BODY,
                have: body.len(),
                what: "ScanAck",
            });
        }
        Ok(Self {
            ts_state: body[0],
            tuning_information_message: &body[1..1 + TUNING_INFO_MESSAGE_LEN],
            scan_progress: body[1 + TUNING_INFO_MESSAGE_LEN],
        })
    }
}
impl Serialize for ScanAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(SCAN_ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.tuning_information_message.len() != TUNING_INFO_MESSAGE_LEN {
            return Err(Error::InvalidObject {
                what: "ScanAck",
                reason: "TuningInformationMessage must be exactly 11 bytes",
            });
        }
        let mut pos = objects::write_apdu_header(tag::SCAN_ACK, SCAN_ACK_BODY, buf)?;
        buf[pos] = self.ts_state;
        pos += 1;
        buf[pos..pos + TUNING_INFO_MESSAGE_LEN].copy_from_slice(self.tuning_information_message);
        pos += TUNING_INFO_MESSAGE_LEN;
        buf[pos] = self.scan_progress;
        Ok(pos + 1)
    }
}

// --- TuneTSReq ---

impl<'a> Parse<'a> for TuneTSReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::TUNE_TS_REQ, "TuneTSReq")?;
        // The TuningInformationMessage is either absent (disconnect) or 11 bytes.
        if !body.is_empty() && body.len() != TUNING_INFO_MESSAGE_LEN {
            return Err(Error::InvalidObject {
                what: "TuneTSReq",
                reason: "TuningInformationMessage must be absent or exactly 11 bytes",
            });
        }
        Ok(Self {
            tuning_information_message: body,
        })
    }
}
impl Serialize for TuneTSReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.tuning_information_message.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.tuning_information_message.is_empty()
            && self.tuning_information_message.len() != TUNING_INFO_MESSAGE_LEN
        {
            return Err(Error::InvalidObject {
                what: "TuneTSReq",
                reason: "TuningInformationMessage must be absent or exactly 11 bytes",
            });
        }
        let body_len = self.tuning_information_message.len();
        let pos = objects::write_apdu_header(tag::TUNE_TS_REQ, body_len, buf)?;
        buf[pos..pos + body_len].copy_from_slice(self.tuning_information_message);
        Ok(pos + body_len)
    }
}

// --- TuneTSAck ---

// TSState(1).
const TUNE_TS_ACK_BODY: usize = 1;

impl<'a> Parse<'a> for TuneTSAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::TUNE_TS_ACK, "TuneTSAck")?;
        if body.len() < TUNE_TS_ACK_BODY {
            return Err(Error::BufferTooShort {
                need: TUNE_TS_ACK_BODY,
                have: body.len(),
                what: "TuneTSAck",
            });
        }
        Ok(Self { ts_state: body[0] })
    }
}
impl Serialize for TuneTSAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(TUNE_TS_ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::TUNE_TS_ACK, TUNE_TS_ACK_BODY, buf)?;
        buf[pos] = self.ts_state;
        Ok(pos + TUNE_TS_ACK_BODY)
    }
}

/// Resource-scoped dispatch over the StreamInput objects (Tables 13-20).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StreamInputApdu<'a> {
    /// `DeliverySystemInfoReq` (`9F 80 00`).
    DeliverySystemInfoReq(DeliverySystemInfoReq),
    /// `DeliverySystemInfoAck` (`9F 80 01`).
    DeliverySystemInfoAck(DeliverySystemInfoAck),
    /// `ScanStartReq` (`9F 80 02`).
    ScanStartReq(ScanStartReq),
    /// `ScanNextReq` (`9F 80 03`).
    ScanNextReq(ScanNextReq),
    /// `ScanAck` (`9F 80 04`).
    ScanAck(ScanAck<'a>),
    /// `TuneTSReq` (`9F 80 05`).
    TuneTSReq(TuneTSReq<'a>),
    /// `TuneTSAck` (`9F 80 06`).
    TuneTSAck(TuneTSAck),
}

impl<'a> StreamInputApdu<'a> {
    /// Parse a StreamInput APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "stream_input apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::DELIVERY_SYSTEM_INFO_REQ => Ok(Self::DeliverySystemInfoReq(
                DeliverySystemInfoReq::parse(body)?,
            )),
            tag::DELIVERY_SYSTEM_INFO_ACK => Ok(Self::DeliverySystemInfoAck(
                DeliverySystemInfoAck::parse(body)?,
            )),
            tag::SCAN_START_REQ => Ok(Self::ScanStartReq(ScanStartReq::parse(body)?)),
            tag::SCAN_NEXT_REQ => Ok(Self::ScanNextReq(ScanNextReq::parse(body)?)),
            tag::SCAN_ACK => Ok(Self::ScanAck(ScanAck::parse(body)?)),
            tag::TUNE_TS_REQ => Ok(Self::TuneTSReq(TuneTSReq::parse(body)?)),
            tag::TUNE_TS_ACK => Ok(Self::TuneTSAck(TuneTSAck::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::DELIVERY_SYSTEM_INFO_REQ.as_u24(),
                what: "stream_input",
            }),
        }
    }
}

impl Serialize for StreamInputApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::DeliverySystemInfoReq(o) => o.serialized_len(),
            Self::DeliverySystemInfoAck(o) => o.serialized_len(),
            Self::ScanStartReq(o) => o.serialized_len(),
            Self::ScanNextReq(o) => o.serialized_len(),
            Self::ScanAck(o) => o.serialized_len(),
            Self::TuneTSReq(o) => o.serialized_len(),
            Self::TuneTSAck(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::DeliverySystemInfoReq(o) => o.serialize_into(buf),
            Self::DeliverySystemInfoAck(o) => o.serialize_into(buf),
            Self::ScanStartReq(o) => o.serialize_into(buf),
            Self::ScanNextReq(o) => o.serialize_into(buf),
            Self::ScanAck(o) => o.serialize_into(buf),
            Self::TuneTSReq(o) => o.serialize_into(buf),
            Self::TuneTSAck(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_only_objects_round_trip() {
        assert_eq!(DeliverySystemInfoReq.to_bytes(), [0x9F, 0x80, 0x00, 0x00]);
        assert_eq!(ScanStartReq.to_bytes(), [0x9F, 0x80, 0x02, 0x00]);
        assert_eq!(ScanNextReq.to_bytes(), [0x9F, 0x80, 0x03, 0x00]);
        assert_eq!(
            DeliverySystemInfoReq::parse(&[0x9F, 0x80, 0x00, 0x00]).unwrap(),
            DeliverySystemInfoReq
        );
        assert_eq!(
            ScanNextReq::parse(&[0x9F, 0x80, 0x03, 0x00]).unwrap(),
            ScanNextReq
        );
    }

    #[test]
    fn delivery_system_info_ack_multi_round_trips_and_bites() {
        let ack = DeliverySystemInfoAck {
            systems: alloc::vec![
                SystemIdentifier::DvbC,
                SystemIdentifier::DvbS,
                SystemIdentifier::DvbT,
            ],
        };
        let bytes = ack.to_bytes();
        // tag(3) + len(1) + 3 = 7; body len = 3 = 0x03.
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x03, 0x01, 0x02, 0x03]);
        assert_eq!(DeliverySystemInfoAck::parse(&bytes).unwrap(), ack);
        assert_eq!(ack.systems[0].name(), "DVB-C");
        let mut other = ack.clone();
        other.systems[2] = SystemIdentifier::Abstract;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn delivery_system_info_ack_reserved_value() {
        let ack = DeliverySystemInfoAck {
            systems: alloc::vec![SystemIdentifier::from_u8(0x7F)],
        };
        assert_eq!(ack.systems[0], SystemIdentifier::Reserved(0x7F));
        let bytes = ack.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, 0x7F]);
        assert_eq!(DeliverySystemInfoAck::parse(&bytes).unwrap(), ack);
    }

    #[test]
    fn scan_ack_round_trips_and_bites() {
        let tim = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB,
        ];
        let ack = ScanAck {
            ts_state: 0xC8,
            tuning_information_message: &tim,
            scan_progress: 0x40,
        };
        let bytes = ack.to_bytes();
        // tag(3) + len(1) + state(1) + tim(11) + progress(1) = 17; body = 13 = 0x0D.
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x04, 0x0D, 0xC8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
                0xAA, 0xBB, 0x40
            ]
        );
        assert_eq!(ScanAck::parse(&bytes).unwrap(), ack);
        let mut other = ack.clone();
        other.scan_progress = 0x41;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn tune_ts_req_with_message_round_trips_and_bites() {
        let tim = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        ];
        let req = TuneTSReq {
            tuning_information_message: &tim,
        };
        let bytes = req.to_bytes();
        assert_eq!(
            bytes,
            [
                0x9F, 0x80, 0x05, 0x0B, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A,
                0x0B
            ]
        );
        assert_eq!(TuneTSReq::parse(&bytes).unwrap(), req);
        let mut tim2 = tim;
        tim2[10] = 0xFF;
        let other = TuneTSReq {
            tuning_information_message: &tim2,
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn tune_ts_req_disconnect_empty_message() {
        let req = TuneTSReq {
            tuning_information_message: &[],
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x05, 0x00]);
        assert_eq!(TuneTSReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn tune_ts_req_rejects_wrong_length() {
        // 5-byte TuningInformationMessage is neither absent nor 11 bytes.
        let bad = [0x9F, 0x80, 0x05, 0x05, 0x01, 0x02, 0x03, 0x04, 0x05];
        assert!(matches!(
            TuneTSReq::parse(&bad),
            Err(Error::InvalidObject { .. })
        ));
    }

    #[test]
    fn tune_ts_ack_round_trips_and_bites() {
        let ack = TuneTSAck { ts_state: 0xFF };
        let bytes = ack.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x06, 0x01, 0xFF]);
        assert_eq!(TuneTSAck::parse(&bytes).unwrap(), ack);
        let other = TuneTSAck { ts_state: 0x00 };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let req = DeliverySystemInfoReq.to_bytes();
        assert!(matches!(
            StreamInputApdu::parse(&req).unwrap(),
            StreamInputApdu::DeliverySystemInfoReq(_)
        ));
        let ack = TuneTSAck { ts_state: 1 }.to_bytes();
        let parsed = StreamInputApdu::parse(&ack).unwrap();
        assert!(matches!(parsed, StreamInputApdu::TuneTSAck(_)));
        assert_eq!(parsed.to_bytes(), ack);
        // ScanNextReq is 9F8003 (Table 17) — distinct from ScanStartReq 9F8002.
        let next = ScanNextReq.to_bytes();
        assert!(matches!(
            StreamInputApdu::parse(&next).unwrap(),
            StreamInputApdu::ScanNextReq(_)
        ));
    }
}
