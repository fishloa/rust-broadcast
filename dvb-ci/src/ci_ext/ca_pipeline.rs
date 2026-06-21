//! CA Pipeline objects — ETSI TS 101 699 V1.1.1 §6.8, Tables 84-86
//! (PDF pp. 75-77). See `docs/ci_plus/ca-pipeline.md`.
//!
//! Resource ID `0x00061ii1` (`ii` = Module ID, `type = 1*`). A module-provided
//! framework that lets receiver-hosted applications and CA systems exchange
//! CA-system-specific messages. The `CASpecificData` byte string is **opaque**
//! (its encoding is a matter for the application-domain specification that
//! invokes this interface) and is carried verbatim as a borrowed `&[u8]`.
//!
//! - `CAPipelineRequest` (`9F 80 00`, Table 84) — host app → module.
//! - `CAPipelineResponse` (`9F 80 01`, Table 85) — module → app.
//! - `CAPipelineNotification` (`9F 80 02`, Table 86) — module → app, asynchronous.

use crate::error::{Error, Result};
use crate::objects;
use crate::tag::ApduTag;
use dvb_common::{Parse, Serialize};

/// Resource-scoped `apdu_tag`s for CA Pipeline (Tables 84-86).
pub mod tag {
    use crate::tag::ApduTag;
    /// `CAPRequestTag` = `9F 80 00`.
    pub const CAP_REQUEST: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x00);
    /// `CAP_response_tag` = `9F 80 01`.
    pub const CAP_RESPONSE: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x01);
    /// `CAPNotificationTag` = `9F 80 02`.
    pub const CAP_NOTIFICATION: ApduTag = ApduTag::from_bytes(0x9F, 0x80, 0x02);
}

/// `CAPRequest()` (Table 84): host application → module. The `CASpecificData`
/// is an opaque, CA-system-specific byte blob.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPipelineRequest<'a> {
    /// Opaque `CASpecificData` bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub ca_specific_data: &'a [u8],
}

/// `CAPResponse()` (Table 85): module → application. Identical shape to
/// [`CaPipelineRequest`] except for the tag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPipelineResponse<'a> {
    /// Opaque `CA_specific_Data` bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub ca_specific_data: &'a [u8],
}

/// `CAPNotification()` (Table 86): module → application, asynchronous. Carries
/// optional opaque event data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CaPipelineNotification<'a> {
    /// Opaque `CASpecificData` bytes.
    #[cfg_attr(feature = "serde", serde(borrow, with = "crate::objects::bytes_serde"))]
    pub ca_specific_data: &'a [u8],
}

macro_rules! opaque_object {
    ($ty:ident, $tag:expr, $what:literal) => {
        impl<'a> Parse<'a> for $ty<'a> {
            type Error = Error;
            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let body = objects::parse_apdu_header(bytes, $tag, $what)?;
                Ok(Self {
                    ca_specific_data: body,
                })
            }
        }
        impl Serialize for $ty<'_> {
            type Error = Error;
            fn serialized_len(&self) -> usize {
                objects::apdu_len(self.ca_specific_data.len())
            }
            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let body_len = self.ca_specific_data.len();
                let pos = objects::write_apdu_header($tag, body_len, buf)?;
                buf[pos..pos + body_len].copy_from_slice(self.ca_specific_data);
                Ok(pos + body_len)
            }
        }
    };
}

opaque_object!(CaPipelineRequest, tag::CAP_REQUEST, "CAPipelineRequest");
opaque_object!(CaPipelineResponse, tag::CAP_RESPONSE, "CAPipelineResponse");
opaque_object!(
    CaPipelineNotification,
    tag::CAP_NOTIFICATION,
    "CAPipelineNotification"
);

/// Resource-scoped dispatch over the CA Pipeline objects (Tables 84-86).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CaPipelineApdu<'a> {
    /// `CAPipelineRequest` (`9F 80 00`).
    Request(CaPipelineRequest<'a>),
    /// `CAPipelineResponse` (`9F 80 01`).
    Response(CaPipelineResponse<'a>),
    /// `CAPipelineNotification` (`9F 80 02`).
    Notification(CaPipelineNotification<'a>),
}

impl<'a> CaPipelineApdu<'a> {
    /// Parse a CA Pipeline APDU, dispatching on the leading `apdu_tag`.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        if body.len() < 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: body.len(),
                what: "ca_pipeline apdu_tag",
            });
        }
        let t = ApduTag::from_bytes(body[0], body[1], body[2]);
        match t {
            tag::CAP_REQUEST => Ok(Self::Request(CaPipelineRequest::parse(body)?)),
            tag::CAP_RESPONSE => Ok(Self::Response(CaPipelineResponse::parse(body)?)),
            tag::CAP_NOTIFICATION => Ok(Self::Notification(CaPipelineNotification::parse(body)?)),
            _ => Err(Error::UnexpectedApduTag {
                got: t.as_u24(),
                expected: tag::CAP_REQUEST.as_u24(),
                what: "ca_pipeline",
            }),
        }
    }
}

impl Serialize for CaPipelineApdu<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::Request(o) => o.serialized_len(),
            Self::Response(o) => o.serialized_len(),
            Self::Notification(o) => o.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Request(o) => o.serialize_into(buf),
            Self::Response(o) => o.serialize_into(buf),
            Self::Notification(o) => o.serialize_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_and_bites() {
        let req = CaPipelineRequest {
            ca_specific_data: &[0xAA, 0xBB, 0xCC],
        };
        let bytes = req.to_bytes();
        // tag(3) + len(1=0x03) + 3 data.
        assert_eq!(bytes, [0x9F, 0x80, 0x00, 0x03, 0xAA, 0xBB, 0xCC]);
        assert_eq!(CaPipelineRequest::parse(&bytes).unwrap(), req);
        let other = CaPipelineRequest {
            ca_specific_data: &[0xAA, 0xBB, 0xCD],
        };
        assert_ne!(bytes, other.to_bytes());
    }

    #[test]
    fn response_and_notification_round_trip() {
        let resp = CaPipelineResponse {
            ca_specific_data: &[0x01],
        };
        let bytes = resp.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x01, 0x01, 0x01]);
        assert_eq!(CaPipelineResponse::parse(&bytes).unwrap(), resp);

        let note = CaPipelineNotification {
            ca_specific_data: &[],
        };
        let bytes = note.to_bytes();
        assert_eq!(bytes, [0x9F, 0x80, 0x02, 0x00]);
        assert_eq!(CaPipelineNotification::parse(&bytes).unwrap(), note);
    }

    #[test]
    fn dispatch_routes_each_tag() {
        let req = CaPipelineRequest {
            ca_specific_data: &[0x10],
        }
        .to_bytes();
        assert!(matches!(
            CaPipelineApdu::parse(&req).unwrap(),
            CaPipelineApdu::Request(_)
        ));
        let resp = CaPipelineResponse {
            ca_specific_data: &[0x20],
        }
        .to_bytes();
        assert!(matches!(
            CaPipelineApdu::parse(&resp).unwrap(),
            CaPipelineApdu::Response(_)
        ));
        let note = CaPipelineNotification {
            ca_specific_data: &[0x30],
        }
        .to_bytes();
        let parsed = CaPipelineApdu::parse(&note).unwrap();
        assert!(matches!(parsed, CaPipelineApdu::Notification(_)));
        assert_eq!(parsed.to_bytes(), note);
    }

    #[test]
    fn unexpected_tag_errors() {
        let bad = [0x9F, 0x80, 0x09, 0x00];
        assert!(matches!(
            CaPipelineApdu::parse(&bad),
            Err(Error::UnexpectedApduTag { .. })
        ));
    }
}
