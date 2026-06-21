//! Content Control resource — multi-stream extensions — ETSI TS 103 205 V1.4.1
//! §6.4.3, Tables 6-13 (PDF pp. 34-38). See `docs/ts_103_205/content-control.md`.
//!
//! Resource ID `0x008C1041` (Class 140, Type 65, Version 1). For multi-stream
//! reception the Content Control APDUs are extended with the Local TS identifier
//! (`LTS_id`). Only the two extended APDUs whose syntax TS 103 205 actually
//! prints are encoded here:
//!
//! - `cc_PIN_reply` (`9F 90 14`, Table 7) — CICAM → Host.
//! - `cc_PIN_event` (`9F 90 15`, Table 8) — CICAM → Host.
//!
//! The remaining Table 6 APDUs (`cc_open_*`, `cc_data_*`, `cc_sync_*`,
//! `cc_sac_*`, `cc_PIN_capabilities_*`, `cc_PIN_cmd`, `cc_PIN_playback`,
//! `cc_PIN_MMI_req`) defer to CI Plus V1.3 \[3\] §11.3.x and are **not encoded**.
//!
//! §6.4.3.3 (Tables 9-13) specifies the SAC protocol-message *content* — the
//! ordered datatype loops carried inside the `cc_sac_data_req`/`cc_sac_data_cnf`
//! envelope. These are modelled as [`SacMessage`] / [`SacDatatype`] (the
//! envelope APDU itself defers to CI Plus V1.3, so no `9F9007`/`9F9008` APDU is
//! produced here). Crypto / license / PIN payload bytes are carried as opaque
//! borrowed slices.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use alloc::vec::Vec;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s implemented for the Content Control resource
/// (the printed-syntax subset of Table 6).
pub mod tag {
    use crate::tag::ApduTag;
    /// `cc_PIN_reply_tag` = `9F 90 14`.
    pub const CC_PIN_REPLY: ApduTag = ApduTag::from_bytes(0x9F, 0x90, 0x14);
    /// `cc_PIN_event_tag` = `9F 90 15`.
    pub const CC_PIN_EVENT: ApduTag = ApduTag::from_bytes(0x9F, 0x90, 0x15);
}

// --- cc_PIN_reply (Table 7) ---

/// `cc_PIN_reply()` (Table 7): CICAM → Host. Extended for the record-start
/// protocol to optionally carry `LTS_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CcPinReply {
    /// `LTS_id` (8) when `LTS_bound_flag == 1`; `None` (the `else` reserved(8)
    /// branch) when the reply is not associated with a Local TS.
    pub lts_id: Option<u8>,
    /// `PINcode_status_field` (8) — semantics in CI Plus V1.3 \[3\] §11.3.2.3
    /// (proprietary, carried verbatim).
    pub pincode_status: u8,
}

// reserved(7)+LTS_bound_flag(1) byte + LTS_id-or-reserved(1) + PINcode_status(1).
const CC_PIN_REPLY_BODY: usize = 3;
// The LTS_bound_flag bit (LSB of the reserved(7)+flag(1) byte).
const LTS_BOUND_FLAG_BIT: u8 = 0x01;

impl<'a> Parse<'a> for CcPinReply {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CC_PIN_REPLY, "cc_PIN_reply")?;
        if body.len() < CC_PIN_REPLY_BODY {
            return Err(Error::BufferTooShort {
                need: CC_PIN_REPLY_BODY,
                have: body.len(),
                what: "cc_PIN_reply",
            });
        }
        let lts_bound = body[0] & LTS_BOUND_FLAG_BIT != 0;
        let lts_id = if lts_bound { Some(body[1]) } else { None };
        Ok(Self {
            lts_id,
            pincode_status: body[2],
        })
    }
}
impl Serialize for CcPinReply {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CC_PIN_REPLY_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::CC_PIN_REPLY, CC_PIN_REPLY_BODY, buf)?;
        match self.lts_id {
            Some(id) => {
                buf[pos] = LTS_BOUND_FLAG_BIT;
                buf[pos + 1] = id;
            }
            None => {
                buf[pos] = 0;
                // else branch: reserved(8).
                buf[pos + 1] = 0;
            }
        }
        buf[pos + 2] = self.pincode_status;
        Ok(pos + CC_PIN_REPLY_BODY)
    }
}

// --- cc_PIN_event (Table 8) ---

/// Width of the `private_data` field (Table 8: `8x15` bits = 15 bytes).
pub const PIN_EVENT_PRIVATE_DATA_LEN: usize = 15;

/// `cc_PIN_event()` (Table 8): CICAM → Host. Extended for the record-start
/// protocol to include `LTS_id`. The field meanings (other than `LTS_id`) are in
/// CI Plus V1.3 \[3\] §11.3.2.4 (proprietary) and carried verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CcPinEvent {
    /// `LTS_id` (8) — Local TS identifier.
    pub lts_id: u8,
    /// `program_number` (16).
    pub program_number: u16,
    /// `PINcode_status_field` (8).
    pub pincode_status: u8,
    /// `rating` (8).
    pub rating: u8,
    /// `pin_event_time_utc` (40) — held in the low 5 bytes of the `u64`.
    pub pin_event_time_utc: u64,
    /// `pin_event_time_centiseconds` (8).
    pub pin_event_time_centiseconds: u8,
    /// `private_data` (`8x15` = 15 bytes).
    pub private_data: [u8; PIN_EVENT_PRIVATE_DATA_LEN],
}

// LTS_id(1)+program_number(2)+PINcode_status(1)+rating(1)+utc(5)+centi(1)+private(15).
const CC_PIN_EVENT_BODY: usize = 1 + 2 + 1 + 1 + 5 + 1 + PIN_EVENT_PRIVATE_DATA_LEN;

impl<'a> Parse<'a> for CcPinEvent {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::CC_PIN_EVENT, "cc_PIN_event")?;
        if body.len() < CC_PIN_EVENT_BODY {
            return Err(Error::BufferTooShort {
                need: CC_PIN_EVENT_BODY,
                have: body.len(),
                what: "cc_PIN_event",
            });
        }
        // Layout (Table 8): LTS_id(1) program_number(2) PINcode_status(1)
        // rating(1) pin_event_time_utc(5) pin_event_time_centiseconds(1)
        // private_data(15).
        let mut utc = 0u64;
        for &b in &body[5..10] {
            utc = (utc << 8) | b as u64;
        }
        let mut private_data = [0u8; PIN_EVENT_PRIVATE_DATA_LEN];
        private_data.copy_from_slice(&body[11..11 + PIN_EVENT_PRIVATE_DATA_LEN]);
        Ok(Self {
            lts_id: body[0],
            program_number: u16::from_be_bytes([body[1], body[2]]),
            pincode_status: body[3],
            rating: body[4],
            pin_event_time_utc: utc,
            pin_event_time_centiseconds: body[10],
            private_data,
        })
    }
}
impl Serialize for CcPinEvent {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(CC_PIN_EVENT_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::CC_PIN_EVENT, CC_PIN_EVENT_BODY, buf)?;
        buf[pos] = self.lts_id;
        buf[pos + 1..pos + 3].copy_from_slice(&self.program_number.to_be_bytes());
        buf[pos + 3] = self.pincode_status;
        buf[pos + 4] = self.rating;
        // utc(40) — low 5 bytes, big-endian.
        let utc = self.pin_event_time_utc.to_be_bytes();
        buf[pos + 5..pos + 10].copy_from_slice(&utc[3..8]);
        buf[pos + 10] = self.pin_event_time_centiseconds;
        buf[pos + 11..pos + 11 + PIN_EVENT_PRIVATE_DATA_LEN].copy_from_slice(&self.private_data);
        Ok(pos + CC_PIN_EVENT_BODY)
    }
}

// --- SAC protocol datatypes (§6.4.3.3, Tables 9-13) ---

/// `datatype_id` of a SAC protocol datatype (§6.4.3.3, Tables 9-13 + the new
/// multi-stream `LTS_id` = 50). The named values are those referenced by the
/// TS 103 205 protocol tables; other values are [`DatatypeId::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DatatypeId {
    /// `uri_message` (25).
    UriMessage,
    /// `program_number` (26).
    ProgramNumber,
    /// `uri_confirm` (27).
    UriConfirm,
    /// `cicam_license` (33) — opaque license blob.
    CicamLicense,
    /// `license_status` (34).
    LicenseStatus,
    /// `license_rcvd_status` (35).
    LicenseRcvdStatus,
    /// `operating_mode` (38).
    OperatingMode,
    /// `PINcode` data (39) — opaque.
    PinCode,
    /// `record_start_status` (40).
    RecordStartStatus,
    /// `mode_change_status` (41).
    ModeChangeStatus,
    /// `record_stop_status` (42).
    RecordStopStatus,
    /// `LTS_id` (50) — the new multi-stream datatype.
    LtsId,
    /// Any other `datatype_id`.
    Other(u8),
}

impl DatatypeId {
    /// Decode a `datatype_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            25 => Self::UriMessage,
            26 => Self::ProgramNumber,
            27 => Self::UriConfirm,
            33 => Self::CicamLicense,
            34 => Self::LicenseStatus,
            35 => Self::LicenseRcvdStatus,
            38 => Self::OperatingMode,
            39 => Self::PinCode,
            40 => Self::RecordStartStatus,
            41 => Self::ModeChangeStatus,
            42 => Self::RecordStopStatus,
            50 => Self::LtsId,
            other => Self::Other(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::UriMessage => 25,
            Self::ProgramNumber => 26,
            Self::UriConfirm => 27,
            Self::CicamLicense => 33,
            Self::LicenseStatus => 34,
            Self::LicenseRcvdStatus => 35,
            Self::OperatingMode => 38,
            Self::PinCode => 39,
            Self::RecordStartStatus => 40,
            Self::ModeChangeStatus => 41,
            Self::RecordStopStatus => 42,
            Self::LtsId => 50,
            Self::Other(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UriMessage => "uri_message",
            Self::ProgramNumber => "program_number",
            Self::UriConfirm => "uri_confirm",
            Self::CicamLicense => "cicam_license",
            Self::LicenseStatus => "license_status",
            Self::LicenseRcvdStatus => "license_rcvd_status",
            Self::OperatingMode => "operating_mode",
            Self::PinCode => "PINcode",
            Self::RecordStartStatus => "record_start_status",
            Self::ModeChangeStatus => "mode_change_status",
            Self::RecordStopStatus => "record_stop_status",
            Self::LtsId => "LTS_id",
            Self::Other(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(DatatypeId, Other);

/// `operating_mode` (datatype 38) values (§6.4.3.3.2 prose): the CICAM treats
/// the programme as unattended when this is `Timeshift` or `UnattendedRecording`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum OperatingMode {
    /// `0x01` — Timeshift.
    Timeshift,
    /// `0x02` — Unattended Recording.
    UnattendedRecording,
    /// Any other value (reserved / attended modes are CI Plus V1.3 defined).
    Other(u8),
}

impl OperatingMode {
    /// Decode an `operating_mode` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Timeshift,
            0x02 => Self::UnattendedRecording,
            other => Self::Other(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Timeshift => 0x01,
            Self::UnattendedRecording => 0x02,
            Self::Other(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Timeshift => "Timeshift",
            Self::UnattendedRecording => "Unattended_Recording",
            Self::Other(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(OperatingMode, Other);

/// A single SAC protocol datatype: `datatype_id` (8) + `datatype_length` (16,
/// number of value bytes) + value. Value bytes are carried opaque so crypto /
/// license / PIN payloads round-trip verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SacDatatype<'a> {
    /// `datatype_id` (8).
    pub datatype_id: DatatypeId,
    /// `datatype` value bytes (length given by the 16-bit `datatype_length`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub value: &'a [u8],
}

// datatype_id(1) + datatype_length(2).
const SAC_DATATYPE_HEADER: usize = 3;

impl<'a> SacDatatype<'a> {
    /// Serialized length of this datatype (header + value).
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        SAC_DATATYPE_HEADER + self.value.len()
    }
}

/// A SAC protocol message (§6.4.3.3, Tables 9-13): the ordered loop of datatypes
/// carried inside a `cc_sac_data_req` / `cc_sac_data_cnf` envelope. The envelope
/// APDU itself (`9F9007`/`9F9008`) defers to CI Plus V1.3, so this models only the
/// datatype loop content.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SacMessage<'a> {
    /// The ordered datatypes (the `send`/`request` datatype loop content).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub datatypes: Vec<SacDatatype<'a>>,
}

impl<'a> SacMessage<'a> {
    /// Parse a contiguous SAC datatype loop (`datatype_nbr` entries are inferred
    /// from the buffer; the buffer must contain exactly whole datatypes).
    pub fn parse(mut body: &'a [u8]) -> Result<Self> {
        let mut datatypes = Vec::new();
        while !body.is_empty() {
            if body.len() < SAC_DATATYPE_HEADER {
                return Err(Error::BufferTooShort {
                    need: SAC_DATATYPE_HEADER,
                    have: body.len(),
                    what: "SAC datatype header",
                });
            }
            let datatype_id = DatatypeId::from_u8(body[0]);
            let len = u16::from_be_bytes([body[1], body[2]]) as usize;
            let end = SAC_DATATYPE_HEADER + len;
            if body.len() < end {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: body.len(),
                    what: "SAC datatype value",
                });
            }
            datatypes.push(SacDatatype {
                datatype_id,
                value: &body[SAC_DATATYPE_HEADER..end],
            });
            body = &body[end..];
        }
        Ok(Self { datatypes })
    }

    /// Serialized length of the whole datatype loop.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        self.datatypes.iter().map(SacDatatype::serialized_len).sum()
    }

    /// Serialize the datatype loop into `buf`, returning the bytes written.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        let mut pos = 0;
        for dt in &self.datatypes {
            if dt.value.len() > u16::MAX as usize {
                return Err(Error::LengthTooLarge(dt.value.len()));
            }
            buf[pos] = dt.datatype_id.to_u8();
            buf[pos + 1..pos + 3].copy_from_slice(&(dt.value.len() as u16).to_be_bytes());
            pos += SAC_DATATYPE_HEADER;
            buf[pos..pos + dt.value.len()].copy_from_slice(dt.value);
            pos += dt.value.len();
        }
        Ok(pos)
    }

    /// Serialize the datatype loop to a `Vec`.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        let n = self.serialize_into(&mut buf).expect("buffer sized exactly");
        debug_assert_eq!(n, buf.len());
        buf
    }
}

/// Resource-scoped dispatch over the printed-syntax Content Control objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ContentControlApdu {
    /// `cc_PIN_reply` (`9F 90 14`).
    CcPinReply(CcPinReply),
    /// `cc_PIN_event` (`9F 90 15`).
    CcPinEvent(CcPinEvent),
}

impl ContentControlApdu {
    /// Parse a Content Control APDU, dispatching on the leading `apdu_tag`.
    ///
    /// Only the two extended APDUs whose syntax TS 103 205 prints
    /// (`cc_PIN_reply` / `cc_PIN_event`) are recognized; all other Table 6 tags
    /// defer to CI Plus V1.3 and yield [`Error::UnexpectedApduTag`].
    pub fn parse(body: &[u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "content_control apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::CC_PIN_REPLY => Ok(Self::CcPinReply(CcPinReply::parse(body)?)),
            tag::CC_PIN_EVENT => Ok(Self::CcPinEvent(CcPinEvent::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::CC_PIN_REPLY.as_u24(),
                what: "content_control",
            }),
        }
    }
}

impl Serialize for ContentControlApdu {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::CcPinReply(o) => o.serialized_len(),
            Self::CcPinEvent(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::CcPinReply(o) => o.serialize_into(buf),
            Self::CcPinEvent(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_pin_reply_bound_round_trips_and_bites() {
        let r = CcPinReply {
            lts_id: Some(0x07),
            pincode_status: 0x42,
        };
        let bytes = r.to_bytes();
        // tag(3) + len(0x03) + flag-byte(0x01) + LTS_id(0x07) + status(0x42).
        assert_eq!(bytes, [0x9F, 0x90, 0x14, 0x03, 0x01, 0x07, 0x42]);
        assert_eq!(CcPinReply::parse(&bytes).unwrap(), r);
        // Field-mutation: dropping the LTS binding changes the wire.
        let other = CcPinReply {
            lts_id: None,
            pincode_status: 0x42,
        };
        let ob = other.to_bytes();
        assert_eq!(ob, [0x9F, 0x90, 0x14, 0x03, 0x00, 0x00, 0x42]);
        assert_ne!(bytes, ob);
        assert_eq!(CcPinReply::parse(&ob).unwrap(), other);
    }

    #[test]
    fn cc_pin_event_round_trips_and_bites() {
        let e = CcPinEvent {
            lts_id: 0x03,
            program_number: 0x1234,
            pincode_status: 0x05,
            rating: 0x0A,
            pin_event_time_utc: 0x01_0203_0405,
            pin_event_time_centiseconds: 0x63,
            private_data: [
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
                0x1E,
            ],
        };
        let bytes = e.to_bytes();
        let expected = [
            0x9F, 0x90, 0x15, 0x1A, // tag + len (0x1A = 26 body bytes)
            0x03, // LTS_id
            0x12, 0x34, // program_number
            0x05, // PINcode_status
            0x0A, // rating
            0x01, 0x02, 0x03, 0x04, 0x05, // utc (40 bits)
            0x63, // centiseconds
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D,
            0x1E, // private_data (15)
        ];
        assert_eq!(bytes, expected);
        assert_eq!(CcPinEvent::parse(&bytes).unwrap(), e);
        // Field-mutation: bump utc.
        let mut other = e;
        other.pin_event_time_utc += 1;
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn sac_message_two_datatypes_round_trips() {
        // Record Stop content (Table 11): program_number(26)+LTS_id(50).
        let msg = SacMessage {
            datatypes: alloc::vec![
                SacDatatype {
                    datatype_id: DatatypeId::ProgramNumber,
                    value: &[0x12, 0x34],
                },
                SacDatatype {
                    datatype_id: DatatypeId::LtsId,
                    value: &[0x07],
                },
            ],
        };
        let bytes = msg.to_bytes();
        // 26=0x1A len 0x0002 12 34 ; 50=0x32 len 0x0001 07
        assert_eq!(
            bytes,
            [0x1A, 0x00, 0x02, 0x12, 0x34, 0x32, 0x00, 0x01, 0x07]
        );
        assert_eq!(SacMessage::parse(&bytes).unwrap(), msg);
        // Field-mutation: change a datatype_id.
        let mut other = msg.clone();
        other.datatypes[1].datatype_id = DatatypeId::Other(99);
        assert_ne!(bytes, other.to_bytes());
        assert_eq!(other.to_bytes()[5], 99);
    }

    #[test]
    fn sac_message_with_opaque_license() {
        // License Exchange (Table 13) subset: cicam_license(33) opaque blob.
        let msg = SacMessage {
            datatypes: alloc::vec![SacDatatype {
                datatype_id: DatatypeId::CicamLicense,
                value: &[0xDE, 0xAD, 0xBE, 0xEF],
            }],
        };
        let bytes = msg.to_bytes();
        assert_eq!(bytes, [0x21, 0x00, 0x04, 0xDE, 0xAD, 0xBE, 0xEF]);
        let parsed = SacMessage::parse(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(parsed.datatypes[0].datatype_id.name(), "cicam_license");
    }

    #[test]
    fn datatype_id_and_operating_mode_labels() {
        assert_eq!(DatatypeId::LtsId.to_u8(), 50);
        assert_eq!(DatatypeId::from_u8(50), DatatypeId::LtsId);
        assert_eq!(DatatypeId::from_u8(33), DatatypeId::CicamLicense);
        assert_eq!(DatatypeId::from_u8(39), DatatypeId::PinCode);
        assert_eq!(DatatypeId::PinCode.name(), "PINcode");
        assert_eq!(DatatypeId::Other(7).name(), "reserved");
        assert_eq!(OperatingMode::from_u8(0x01), OperatingMode::Timeshift);
        assert_eq!(
            OperatingMode::from_u8(0x02),
            OperatingMode::UnattendedRecording
        );
        assert_eq!(OperatingMode::Timeshift.to_u8(), 0x01);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let r = CcPinReply {
            lts_id: None,
            pincode_status: 0,
        }
        .to_bytes();
        let parsed = ContentControlApdu::parse(&r).unwrap();
        assert!(matches!(parsed, ContentControlApdu::CcPinReply(_)));
        assert_eq!(parsed.to_bytes(), r);
        // An unprinted Table 6 tag (cc_open_req 9F9001) is not encoded here.
        let cc_open = [0x9F, 0x90, 0x01, 0x00];
        assert!(matches!(
            ContentControlApdu::parse(&cc_open),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
