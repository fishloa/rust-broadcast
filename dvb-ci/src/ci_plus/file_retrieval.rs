//! Auxiliary File System resource (CICAM file retrieval) — ETSI TS 103 205
//! V1.4.1 §9, Tables 72-75 (PDF pp. 96-99). See `docs/ts_103_205/file-retrieval.md`.
//!
//! Resource ID `0x00910041` (Class 145, Type 1, Version 1). A generic read-only
//! mechanism for a CICAM to offer files to the Host (including CICAM broadcast
//! applications to launch). Tags live in the CI Plus `0x9F94xx` namespace.
//!
//! - `FileSystemOffer` (`9F 94 00`, Table 72) — CICAM → Host.
//! - `FileSystemAck` (`9F 94 01`, Table 73) — Host → CICAM.
//! - `FileRequest` (`9F 94 02`, §9.4) — Host → CICAM.
//! - `FileAcknowledge` (`9F 94 03`, §9.5) — CICAM → Host.
//!
//! ## Deferred bodies (`FileRequest` / `FileAcknowledge`)
//!
//! TS 103 205 §9.4/§9.5 establish only the **tags and direction** of
//! `FileRequest` (`0x9F9402`) and `FileAcknowledge` (`0x9F9403`); their syntax is
//! by reference to CI Plus V1.3 \[3\] §14.5.1 / §14.5.2 (proprietary, not
//! reproduced). We therefore model both as opaque header-only APDUs carrying their
//! body verbatim as borrowed `&[u8]` ([`FileRequest`] / [`FileAcknowledge`]) — the
//! field layout is **not invented**. A caller that knows the V1.3 body shape can
//! decode `body` itself.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for the Auxiliary File System resource (Table 75).
pub mod tag {
    use crate::tag::ApduTag;
    /// `FilesystemOffer_tag` = `9F 94 00`.
    pub const FILE_SYSTEM_OFFER: ApduTag = ApduTag::from_bytes(0x9F, 0x94, 0x00);
    /// `FilesystemAck_tag` = `9F 94 01`.
    pub const FILE_SYSTEM_ACK: ApduTag = ApduTag::from_bytes(0x9F, 0x94, 0x01);
    /// `FileRequest_tag` = `9F 94 02` (body deferred to CI Plus V1.3 §14.5.1).
    pub const FILE_REQUEST: ApduTag = ApduTag::from_bytes(0x9F, 0x94, 0x02);
    /// `FileAcknowledge_tag` = `9F 94 03` (body deferred to CI Plus V1.3 §14.5.2).
    pub const FILE_ACKNOWLEDGE: ApduTag = ApduTag::from_bytes(0x9F, 0x94, 0x03);
}

// --- AckCode (Table 74) ---

/// `AckCode` values (Table 74), the Host's response to a [`FileSystemOffer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AckCode {
    /// `0x01` — OK, the application environment is supported by the Host.
    Ok,
    /// `0x02` — Unknown DomainIdentifier, not supported by the Host.
    UnknownDomainIdentifier,
    /// Reserved (`0x00`, `0x03`–`0xFF`).
    Reserved(u8),
}
impl AckCode {
    /// Decode an `AckCode` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Ok,
            0x02 => Self::UnknownDomainIdentifier,
            other => Self::Reserved(other),
        }
    }
    /// Wire byte.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Ok => 0x01,
            Self::UnknownDomainIdentifier => 0x02,
            Self::Reserved(v) => v,
        }
    }
    /// Spec token, or `"reserved"`.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::UnknownDomainIdentifier => "unknown_domain_identifier",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(AckCode, Reserved);

// ---------------------------------------------------------------------------
// FileSystemOffer (Table 72)
// ---------------------------------------------------------------------------

/// `FileSystemOffer()` (Table 72): CICAM → Host. Specifies the file system
/// provided by the CICAM.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileSystemOffer<'a> {
    /// `DomainIdentifier` body (`DomainIdentifierLength` bytes) — opaque
    /// middleware-defined identifier (URL / UUID / DVB-registered id).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub domain_identifier: &'a [u8],
}

// DomainIdentifierLength(1) + DomainIdentifier bytes.
const OFFER_PREFIX: usize = 1;

impl<'a> Parse<'a> for FileSystemOffer<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_SYSTEM_OFFER, "FileSystemOffer")?;
        if body.len() < OFFER_PREFIX {
            return Err(Error::BufferTooShort {
                need: OFFER_PREFIX,
                have: body.len(),
                what: "FileSystemOffer",
            });
        }
        let len = body[0] as usize;
        let end = OFFER_PREFIX + len;
        if body.len() < end {
            return Err(Error::LengthMismatch {
                what: "FileSystemOffer DomainIdentifier",
                declared: len,
                actual: body.len().saturating_sub(OFFER_PREFIX),
            });
        }
        Ok(Self {
            domain_identifier: &body[OFFER_PREFIX..end],
        })
    }
}
impl Serialize for FileSystemOffer<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(OFFER_PREFIX + self.domain_identifier.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = OFFER_PREFIX + self.domain_identifier.len();
        let pos = objects::write_apdu_header(tag::FILE_SYSTEM_OFFER, body_len, buf)?;
        buf[pos] = self.domain_identifier.len() as u8;
        buf[pos + OFFER_PREFIX..pos + body_len].copy_from_slice(self.domain_identifier);
        Ok(pos + body_len)
    }
}

// ---------------------------------------------------------------------------
// FileSystemAck (Table 73)
// ---------------------------------------------------------------------------

/// `FileSystemAck()` (Table 73): Host → CICAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileSystemAck {
    /// `AckCode` (8) — Table 74.
    pub ack_code: AckCode,
}

const ACK_BODY: usize = 1;

impl<'a> Parse<'a> for FileSystemAck {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_SYSTEM_ACK, "FileSystemAck")?;
        if body.len() < ACK_BODY {
            return Err(Error::BufferTooShort {
                need: ACK_BODY,
                have: body.len(),
                what: "FileSystemAck",
            });
        }
        Ok(Self {
            ack_code: AckCode::from_u8(body[0]),
        })
    }
}
impl Serialize for FileSystemAck {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(ACK_BODY)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::FILE_SYSTEM_ACK, ACK_BODY, buf)?;
        buf[pos] = self.ack_code.to_u8();
        Ok(pos + ACK_BODY)
    }
}

// ---------------------------------------------------------------------------
// FileRequest (§9.4) / FileAcknowledge (§9.5) — bodies deferred to CI Plus V1.3
// ---------------------------------------------------------------------------

/// `FileRequest()` (§9.4, tag `0x9F9402`): Host → CICAM. **Body deferred** to
/// CI Plus V1.3 \[3\] §14.5.1 (proprietary, not reproduced in TS 103 205) — the
/// body is carried verbatim as opaque borrowed bytes; the field layout is not
/// invented.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileRequest<'a> {
    /// Opaque CI Plus V1.3 §14.5.1 body (whole `length_field()` body, verbatim).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub body: &'a [u8],
}

impl<'a> Parse<'a> for FileRequest<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_REQUEST, "FileRequest")?;
        Ok(Self { body })
    }
}
impl Serialize for FileRequest<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.body.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::FILE_REQUEST, self.body.len(), buf)?;
        buf[pos..pos + self.body.len()].copy_from_slice(self.body);
        Ok(pos + self.body.len())
    }
}

/// `FileAcknowledge()` (§9.5, tag `0x9F9403`): CICAM → Host. **Body deferred** to
/// CI Plus V1.3 \[3\] §14.5.2 (proprietary, not reproduced in TS 103 205) — the
/// body is carried verbatim as opaque borrowed bytes; the field layout is not
/// invented.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileAcknowledge<'a> {
    /// Opaque CI Plus V1.3 §14.5.2 body (whole `length_field()` body, verbatim).
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub body: &'a [u8],
}

impl<'a> Parse<'a> for FileAcknowledge<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = objects::parse_apdu_header(bytes, tag::FILE_ACKNOWLEDGE, "FileAcknowledge")?;
        Ok(Self { body })
    }
}
impl Serialize for FileAcknowledge<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        objects::apdu_len(self.body.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let pos = objects::write_apdu_header(tag::FILE_ACKNOWLEDGE, self.body.len(), buf)?;
        buf[pos..pos + self.body.len()].copy_from_slice(self.body);
        Ok(pos + self.body.len())
    }
}

// ---------------------------------------------------------------------------
// Resource-scoped dispatch
// ---------------------------------------------------------------------------

/// Resource-scoped dispatch over the Auxiliary File System resource objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FileRetrievalApdu<'a> {
    /// `FileSystemOffer` (`9F 94 00`).
    FileSystemOffer(#[cfg_attr(feature = "serde", serde(borrow))] FileSystemOffer<'a>),
    /// `FileSystemAck` (`9F 94 01`).
    FileSystemAck(FileSystemAck),
    /// `FileRequest` (`9F 94 02`) — opaque deferred body.
    FileRequest(#[cfg_attr(feature = "serde", serde(borrow))] FileRequest<'a>),
    /// `FileAcknowledge` (`9F 94 03`) — opaque deferred body.
    FileAcknowledge(#[cfg_attr(feature = "serde", serde(borrow))] FileAcknowledge<'a>),
}

impl<'a> FileRetrievalApdu<'a> {
    /// Parse an Auxiliary File System APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "file_retrieval apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::FILE_SYSTEM_OFFER => Ok(Self::FileSystemOffer(FileSystemOffer::parse(body)?)),
            tag::FILE_SYSTEM_ACK => Ok(Self::FileSystemAck(FileSystemAck::parse(body)?)),
            tag::FILE_REQUEST => Ok(Self::FileRequest(FileRequest::parse(body)?)),
            tag::FILE_ACKNOWLEDGE => Ok(Self::FileAcknowledge(FileAcknowledge::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::FILE_SYSTEM_OFFER.as_u24(),
                what: "file_retrieval",
            }),
        }
    }
}

impl Serialize for FileRetrievalApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::FileSystemOffer(o) => o.serialized_len(),
            Self::FileSystemAck(o) => o.serialized_len(),
            Self::FileRequest(o) => o.serialized_len(),
            Self::FileAcknowledge(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::FileSystemOffer(o) => o.serialize_into(buf),
            Self::FileSystemAck(o) => o.serialize_into(buf),
            Self::FileRequest(o) => o.serialize_into(buf),
            Self::FileAcknowledge(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_round_trips_and_bites() {
        // DomainIdentifier "ab".
        let o = FileSystemOffer {
            domain_identifier: &[0x61, 0x62],
        };
        let bytes = o.to_bytes();
        // tag(9F 94 00) len(03) DomainIdentifierLength(02) 61 62.
        assert_eq!(bytes, [0x9F, 0x94, 0x00, 0x03, 0x02, 0x61, 0x62]);
        assert_eq!(FileSystemOffer::parse(&bytes).unwrap(), o);
        let other = FileSystemOffer {
            domain_identifier: &[0x61, 0x63],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn offer_empty_domain() {
        let o = FileSystemOffer {
            domain_identifier: &[],
        };
        let bytes = o.to_bytes();
        assert_eq!(bytes, [0x9F, 0x94, 0x00, 0x01, 0x00]);
        assert_eq!(FileSystemOffer::parse(&bytes).unwrap(), o);
    }

    #[test]
    fn ack_round_trips() {
        let a = FileSystemAck {
            ack_code: AckCode::UnknownDomainIdentifier,
        };
        let bytes = a.to_bytes();
        assert_eq!(bytes, [0x9F, 0x94, 0x01, 0x01, 0x02]);
        assert_eq!(FileSystemAck::parse(&bytes).unwrap(), a);
        let ok = FileSystemAck {
            ack_code: AckCode::Ok,
        };
        assert_eq!(ok.to_bytes()[4], 0x01);
    }

    #[test]
    fn file_request_opaque_body_round_trips() {
        let r = FileRequest {
            body: &[0x01, 0x02, 0x03],
        };
        let bytes = r.to_bytes();
        assert_eq!(bytes, [0x9F, 0x94, 0x02, 0x03, 0x01, 0x02, 0x03]);
        assert_eq!(FileRequest::parse(&bytes).unwrap(), r);
    }

    #[test]
    fn file_acknowledge_opaque_body_round_trips() {
        let a = FileAcknowledge {
            body: &[0xAA, 0xBB],
        };
        let bytes = a.to_bytes();
        assert_eq!(bytes, [0x9F, 0x94, 0x03, 0x02, 0xAA, 0xBB]);
        assert_eq!(FileAcknowledge::parse(&bytes).unwrap(), a);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let cases: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec![
            FileSystemOffer {
                domain_identifier: &[0x61]
            }
            .to_bytes(),
            FileSystemAck {
                ack_code: AckCode::Ok
            }
            .to_bytes(),
            FileRequest { body: &[0x00] }.to_bytes(),
            FileAcknowledge { body: &[0x00] }.to_bytes(),
        ];
        for c in &cases {
            assert_eq!(&FileRetrievalApdu::parse(c).unwrap().to_bytes(), c);
        }
        assert!(matches!(
            FileRetrievalApdu::parse(&[0x9F, 0x94, 0x7E, 0x00]),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
