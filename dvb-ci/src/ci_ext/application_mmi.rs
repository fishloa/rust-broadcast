//! Application MMI objects — ETSI TS 101 699 V1.1.1 §6.5, Tables 62-68
//! (PDF pp. 57-61). See `docs/ci_plus/application-mmi.md`.
//!
//! Resource ID `0x00410041` (fixed). Lets a module interact with the user by
//! launching an application on the host's application execution environment.
//!
//! - `RequestStart` (`9F 80 00`, Table 62) — app → host: application domain +
//!   initial object (two length-prefixed byte strings).
//! - `RequestStartAck` (`9F 80 01`, Table 63) — host → app: an `AckCode`.
//! - `FileReq` (`9F 80 02`, Table 65) — host → app: a filename byte string.
//! - `FileAck` (`9F 80 03`, Table 66) — app → host: `FileOK` flag + file bytes.
//! - `AppAbortReq` (`9F 80 04`, Table 67) — either direction: opaque abort code.
//! - `AppAbortAck` (`9F 80 05`, Table 68) — response: opaque abort-ack code.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for Application MMI (Tables 62-68).
pub mod tag {
    use crate::tag::ApduTag;
    /// `RequestStartTag` = `9F 80 00`.
    pub const REQUEST_START: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `RequestStartAckTag` = `9F 80 01`.
    pub const REQUEST_START_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `FileReqTag` = `9F 80 02`.
    pub const FILE_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
    /// `FileAckTag` = `9F 80 03`.
    pub const FILE_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x03);
    /// `AppAbortReqTag` = `9F 80 04`.
    pub const APP_ABORT_REQ: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x04);
    /// `AppAbortAckTag` = `9F 80 05`.
    pub const APP_ABORT_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x05);
}

/// `AckCode` — response to a `RequestStart` (Table 64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AckCode {
    /// `0x01` — OK: the execution environment will load and run the initial object.
    Ok,
    /// `0x02` — Wrong API: application domain not supported.
    WrongApi,
    /// `0x03` — API busy: domain supported but not currently available.
    ApiBusy,
    /// `0x80`-`0xFF` — Domain-specific API busy.
    DomainSpecificApiBusy(u8),
    /// `0x00` and `0x04`-`0x7F` — reserved for future use.
    Reserved(u8),
}

impl AckCode {
    /// Decode an `AckCode` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Ok,
            0x02 => Self::WrongApi,
            0x03 => Self::ApiBusy,
            0x80..=0xFF => Self::DomainSpecificApiBusy(v),
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x01,
            Self::WrongApi => 0x02,
            Self::ApiBusy => 0x03,
            Self::DomainSpecificApiBusy(v) | Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::WrongApi => "Wrong API",
            Self::ApiBusy => "API busy",
            Self::DomainSpecificApiBusy(_) => "Domain specific API busy",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(AckCode, DomainSpecificApiBusy, Reserved);

/// `RequestStart()` (Table 62): app → host.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RequestStart<'a> {
    /// `AppDomainIdentifier` — bytes specifying the required application domain.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub app_domain_identifier: &'a [u8],
    /// `InitialObject` — bytes specifying the initial object.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub initial_object: &'a [u8],
}

/// `RequestStartAck()` (Table 63): host → app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RequestStartAck {
    /// `AckCode`.
    pub ack_code: AckCode,
}

/// `FileReq()` (Table 65): host → app — the requested filename bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileReq<'a> {
    /// `FileNameByte`s — the filename requested.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub file_name: &'a [u8],
}

/// `FileAck()` (Table 66): app → host — delivers the requested file (or signals
/// it is unavailable via `file_ok = false`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileAck<'a> {
    /// `FileOK` — `true` if the file is available; `false` otherwise.
    pub file_ok: bool,
    /// `FileByte`s — the file payload (opaque; empty when `file_ok == false`).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub file: &'a [u8],
}

/// `AppAbortReq()` (Table 67): host → app **or** app → host — opaque,
/// application-domain-specific abort qualification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AppAbortReq<'a> {
    /// `AbortReqCode` octet string.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub abort_req_code: &'a [u8],
}

/// `AppAbortAck()` (Table 68): response to `AppAbortReq` — opaque,
/// application-domain-specific abort-ack.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AppAbortAck<'a> {
    /// `AbortAckCode` octet string.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub abort_ack_code: &'a [u8],
}

// --- RequestStart ---

// AppDomainIdentifierLength(1) + InitialObjectLength(1) + the two strings.
const REQUEST_START_FIXED: usize = 2;

impl<'a> Parse<'a> for RequestStart<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::REQUEST_START, "RequestStart")?;
        if body.len() < REQUEST_START_FIXED {
            return Err(Error::BufferTooShort {
                need: REQUEST_START_FIXED,
                have: body.len(),
                what: "RequestStart",
            });
        }
        let domain_len = body[0] as usize;
        let object_len = body[1] as usize;
        let need = REQUEST_START_FIXED + domain_len + object_len;
        if body.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: body.len(),
                what: "RequestStart",
            });
        }
        let domain_start = REQUEST_START_FIXED;
        let object_start = domain_start + domain_len;
        Ok(Self {
            app_domain_identifier: &body[domain_start..object_start],
            initial_object: &body[object_start..object_start + object_len],
        })
    }
}
impl Serialize for RequestStart<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(
            REQUEST_START_FIXED + self.app_domain_identifier.len() + self.initial_object.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let domain_len = self.app_domain_identifier.len();
        let object_len = self.initial_object.len();
        if domain_len > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "RequestStart",
                reason: "AppDomainIdentifier longer than 255 bytes",
            });
        }
        if object_len > u8::MAX as usize {
            return Err(Error::InvalidObject {
                what: "RequestStart",
                reason: "InitialObject longer than 255 bytes",
            });
        }
        let body_len = REQUEST_START_FIXED + domain_len + object_len;
        let mut pos = objects::write_apdu_header(tag::REQUEST_START, body_len, buf)?;
        buf[pos] = domain_len as u8;
        buf[pos + 1] = object_len as u8;
        pos += REQUEST_START_FIXED;
        buf[pos..pos + domain_len].copy_from_slice(self.app_domain_identifier);
        pos += domain_len;
        buf[pos..pos + object_len].copy_from_slice(self.initial_object);
        Ok(pos + object_len)
    }
}

// --- RequestStartAck ---

const REQUEST_START_ACK_BODY: usize = 1;

impl<'a> Parse<'a> for RequestStartAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::REQUEST_START_ACK, "RequestStartAck")?;
        if body.len() < REQUEST_START_ACK_BODY {
            return Err(Error::BufferTooShort {
                need: REQUEST_START_ACK_BODY,
                have: body.len(),
                what: "RequestStartAck",
            });
        }
        Ok(Self {
            ack_code: AckCode::from_u8(body[0]),
        })
    }
}
impl Serialize for RequestStartAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(REQUEST_START_ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::REQUEST_START_ACK, REQUEST_START_ACK_BODY, buf)?;
        buf[pos] = self.ack_code.to_u8();
        Ok(pos + REQUEST_START_ACK_BODY)
    }
}

// --- FileReq ---

impl<'a> Parse<'a> for FileReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_REQ, "FileReq")?;
        Ok(Self { file_name: body })
    }
}
impl Serialize for FileReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.file_name.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.file_name.len();
        let pos = objects::write_apdu_header(tag::FILE_REQ, body_len, buf)?;
        buf[pos..pos + body_len].copy_from_slice(self.file_name);
        Ok(pos + body_len)
    }
}

// --- FileAck ---

/// The `FileOK` flag occupies the low bit of the leading byte; the upper 7 bits
/// are reserved (`0`).
const FILE_OK_BIT: u8 = 0x01;

impl<'a> Parse<'a> for FileAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_ACK, "FileAck")?;
        if body.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "FileAck",
            });
        }
        Ok(Self {
            file_ok: body[0] & FILE_OK_BIT != 0,
            file: &body[1..],
        })
    }
}
impl Serialize for FileAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(1 + self.file.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = 1 + self.file.len();
        let mut pos = objects::write_apdu_header(tag::FILE_ACK, body_len, buf)?;
        buf[pos] = u8::from(self.file_ok);
        pos += 1;
        buf[pos..pos + self.file.len()].copy_from_slice(self.file);
        Ok(pos + self.file.len())
    }
}

// --- AppAbortReq / AppAbortAck (opaque code loops) ---

impl<'a> Parse<'a> for AppAbortReq<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::APP_ABORT_REQ, "AppAbortReq")?;
        Ok(Self {
            abort_req_code: body,
        })
    }
}
impl Serialize for AppAbortReq<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.abort_req_code.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.abort_req_code.len();
        let pos = objects::write_apdu_header(tag::APP_ABORT_REQ, body_len, buf)?;
        buf[pos..pos + body_len].copy_from_slice(self.abort_req_code);
        Ok(pos + body_len)
    }
}

impl<'a> Parse<'a> for AppAbortAck<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::APP_ABORT_ACK, "AppAbortAck")?;
        Ok(Self {
            abort_ack_code: body,
        })
    }
}
impl Serialize for AppAbortAck<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.abort_ack_code.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.abort_ack_code.len();
        let pos = objects::write_apdu_header(tag::APP_ABORT_ACK, body_len, buf)?;
        buf[pos..pos + body_len].copy_from_slice(self.abort_ack_code);
        Ok(pos + body_len)
    }
}

/// Resource-scoped dispatch over the Application MMI objects (Tables 62-68).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ApplicationMmiApdu<'a> {
    /// `RequestStart` (`9F 80 00`).
    RequestStart(RequestStart<'a>),
    /// `RequestStartAck` (`9F 80 01`).
    RequestStartAck(RequestStartAck),
    /// `FileReq` (`9F 80 02`).
    FileReq(FileReq<'a>),
    /// `FileAck` (`9F 80 03`).
    FileAck(FileAck<'a>),
    /// `AppAbortReq` (`9F 80 04`).
    AppAbortReq(AppAbortReq<'a>),
    /// `AppAbortAck` (`9F 80 05`).
    AppAbortAck(AppAbortAck<'a>),
}

impl<'a> ApplicationMmiApdu<'a> {
    /// Parse an Application MMI APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "application_mmi apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::REQUEST_START => Ok(Self::RequestStart(RequestStart::parse(body)?)),
            tag::REQUEST_START_ACK => Ok(Self::RequestStartAck(RequestStartAck::parse(body)?)),
            tag::FILE_REQ => Ok(Self::FileReq(FileReq::parse(body)?)),
            tag::FILE_ACK => Ok(Self::FileAck(FileAck::parse(body)?)),
            tag::APP_ABORT_REQ => Ok(Self::AppAbortReq(AppAbortReq::parse(body)?)),
            tag::APP_ABORT_ACK => Ok(Self::AppAbortAck(AppAbortAck::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::REQUEST_START.as_u24(),
                what: "application_mmi",
            }),
        }
    }
}

impl Serialize for ApplicationMmiApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::RequestStart(o) => o.serialized_len(),
            Self::RequestStartAck(o) => o.serialized_len(),
            Self::FileReq(o) => o.serialized_len(),
            Self::FileAck(o) => o.serialized_len(),
            Self::AppAbortReq(o) => o.serialized_len(),
            Self::AppAbortAck(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::RequestStart(o) => o.serialize_into(buf),
            Self::RequestStartAck(o) => o.serialize_into(buf),
            Self::FileReq(o) => o.serialize_into(buf),
            Self::FileAck(o) => o.serialize_into(buf),
            Self::AppAbortReq(o) => o.serialize_into(buf),
            Self::AppAbortAck(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_start_round_trips_and_bites() {
        let req = RequestStart {
            app_domain_identifier: &[0x01, 0x02, 0x03],
            initial_object: &[0xAA, 0xBB],
        };
        let bytes = req.to_bytes();
        // tag(3) + len(1) + domLen(1=3) + objLen(1=2) + dom(3) + obj(2); body = 7 = 0x07.
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x00, 0x07, 0x03, 0x02, 0x01, 0x02, 0x03, 0xAA, 0xBB]
        );
        assert_eq!(RequestStart::parse(&bytes).unwrap(), req);
        let other = RequestStart {
            app_domain_identifier: &[0x01, 0x02, 0x04],
            initial_object: &[0xAA, 0xBB],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn request_start_empty_strings() {
        let req = RequestStart {
            app_domain_identifier: &[],
            initial_object: &[],
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x02, 0x00, 0x00]);
        assert_eq!(RequestStart::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn request_start_ack_codes() {
        for (byte, code) in [
            (0x01u8, AckCode::Ok),
            (0x02, AckCode::WrongApi),
            (0x03, AckCode::ApiBusy),
            (0x90, AckCode::DomainSpecificApiBusy(0x90)),
            (0x00, AckCode::Reserved(0x00)),
            (0x40, AckCode::Reserved(0x40)),
        ] {
            let ack = RequestStartAck {
                ack_code: AckCode::from_u8(byte),
            };
            assert_eq!(ack.ack_code, code);
            let bytes = ack.to_bytes();
            assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, byte]);
            assert_eq!(RequestStartAck::parse(&bytes).unwrap(), ack);
        }
    }

    #[test]
    fn file_req_round_trips() {
        let req = FileReq {
            file_name: b"app.bin",
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes[..4], [0x9F, 0x80, 0x02, 0x07]);
        assert_eq!(&bytes[4..], b"app.bin");
        assert_eq!(FileReq::parse(&bytes).unwrap(), req);
    }

    #[test]
    fn file_ack_round_trips_and_bites() {
        let ack = FileAck {
            file_ok: true,
            file: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let bytes = ack.to_bytes();
        // tag(3) + len(1=5) + flag(1=0x01) + 4 file bytes.
        assert_eq!(
            bytes,
            [0x9F, 0x80, 0x03, 0x05, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]
        );
        assert_eq!(FileAck::parse(&bytes).unwrap(), ack);
        // file_ok false => flag byte 0x00.
        let unavailable = FileAck {
            file_ok: false,
            file: &[],
        };
        let bytes = unavailable.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x03, 0x01, 0x00]);
        assert_eq!(FileAck::parse(&bytes).unwrap(), unavailable);
        // reserved upper bits ignored on parse (0xFF -> file_ok true).
        let parsed = FileAck::parse(&[0x9F, 0x80, 0x03, 0x01, 0xFF]).unwrap();
        assert!(parsed.file_ok);
    }

    #[test]
    fn app_abort_req_and_ack_round_trip() {
        let req = AppAbortReq {
            abort_req_code: &[0x11, 0x22],
        };
        let bytes = req.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x04, 0x02, 0x11, 0x22]);
        assert_eq!(AppAbortReq::parse(&bytes).unwrap(), req);

        let ack = AppAbortAck {
            abort_ack_code: &[0x33],
        };
        let bytes = ack.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x05, 0x01, 0x33]);
        assert_eq!(AppAbortAck::parse(&bytes).unwrap(), ack);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let rs = RequestStart {
            app_domain_identifier: &[0x01],
            initial_object: &[],
        }
        .to_bytes();
        assert!(matches!(
            ApplicationMmiApdu::parse(&rs).unwrap(),
            ApplicationMmiApdu::RequestStart(_)
        ));
        let aaa = AppAbortAck {
            abort_ack_code: &[0x01],
        }
        .to_bytes();
        let parsed = ApplicationMmiApdu::parse(&aaa).unwrap();
        assert!(matches!(parsed, ApplicationMmiApdu::AppAbortAck(_)));
        assert_eq!(parsed.to_bytes(), aaa);
    }
}
