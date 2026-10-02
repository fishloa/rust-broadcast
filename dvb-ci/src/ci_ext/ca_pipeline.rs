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

use broadcast_common::Parse;

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

crate::dispatch::declare_opaque_apdu!(
    CaPipelineRequest,
    ca_specific_data,
    tag::CAP_REQUEST,
    "CAPipelineRequest"
);
crate::dispatch::declare_opaque_apdu!(
    CaPipelineResponse,
    ca_specific_data,
    tag::CAP_RESPONSE,
    "CAPipelineResponse"
);
crate::dispatch::declare_opaque_apdu!(
    CaPipelineNotification,
    ca_specific_data,
    tag::CAP_NOTIFICATION,
    "CAPipelineNotification"
);

crate::dispatch::declare_resource_apdus! {
    /// Resource-scoped dispatch over the CA Pipeline objects (Tables 84-86).
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    #[non_exhaustive]
    pub enum CaPipelineApdu<'a> ("ca_pipeline") {
        /// `CAPipelineRequest` (`9F 80 00`).
        Request(CaPipelineRequest<'a>) = tag::CAP_REQUEST,
        /// `CAPipelineResponse` (`9F 80 01`).
        Response(CaPipelineResponse<'a>) = tag::CAP_RESPONSE,
        /// `CAPipelineNotification` (`9F 80 02`).
        Notification(CaPipelineNotification<'a>) = tag::CAP_NOTIFICATION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use broadcast_common::Serialize;

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
