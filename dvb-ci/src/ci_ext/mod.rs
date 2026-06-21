//! DVB CI Extensions (ETSI TS 101 699) — the resource-scoped APDU layer.
//!
//! Unlike the EN 50221 application objects (which carry globally-unique
//! `apdu_tag`s and are dispatched by [`crate::AnyApdu`]), the TS 101 699
//! extension resources **reuse the same `0x9F80xx` tag values across different
//! resources** — per Table 87 several resources start their objects at
//! `0x9F8000`. The same three tag bytes therefore mean different objects
//! depending on which resource's session they arrive on, so they cannot join the
//! global `AnyApdu`. This module provides a *resource-scoped* dispatch
//! ([`CiExtApdu`]): parsing is keyed on the `resource_identifier()` first, then
//! the leading 24-bit `apdu_tag` selects the object within that resource.
//!
//! Each `apdu_tag` const lives in its resource module's `tag` submodule, so the
//! colliding values are namespaced per resource (`power_manager::tag` vs
//! `copy_protection::tag` both define a `0x9F8000`).
//!
//! Spec: ETSI TS 101 699 V1.1.1 §8, Table 87 (resource IDs + tags) — see
//! `docs/ci_plus/resource-ids.md`. The per-resource layouts are cited in their
//! own module docs.
//!
//! Resources implemented: Resource Manager v2, Application Information v2, Power
//! Manager, Event Manager, Copy Protection, StreamInput, ServiceGateway (Generic
//! Service Gateway), BroadcastServiceGateway, Status Query (+ audience metering),
//! Application MMI, Download (CAM firmware, + DSM-CC U-N messages), CA Pipeline —
//! the full TS 101 699 §6 resource set.

use crate::error::{Error, Result};
use crate::resource::ResourceId;

pub mod application_info_v2;
pub mod application_mmi;
pub mod broadcast_service_gateway;
pub mod ca_pipeline;
pub mod copy_protection;
pub mod event_manager;
pub mod power_manager;
pub mod resource_manager_v2;
pub mod service_gateway;
pub mod software_download;
pub mod status_query;
pub mod stream_input;

// --- Resource identifiers (TS 101 699 Table 87) ---

/// Resource Manager v2 — class 1, type 1, version 2 (`0x00010042`). Fixed ID.
pub const RESOURCE_MANAGER_V2: ResourceId = ResourceId(0x0001_0042);
/// Application Information v2 — class 2, type 1, version 2 (`0x00020042`). Fixed ID.
pub const APPLICATION_INFO_V2: ResourceId = ResourceId(0x0002_0042);
/// Power Manager — class 34, type 1, version 1 (`0x00220041`). Fixed ID.
pub const POWER_MANAGER: ResourceId = ResourceId(0x0022_0041);
/// Application MMI — class 65, type 1, version 1 (`0x00410041`). Fixed ID.
pub const APPLICATION_MMI: ResourceId = ResourceId(0x0041_0041);
/// Download resource — class 5, type 1, version 1 (`0x00051041`). Fixed ID.
///
/// (Table 87 prints `0x000510041`, a 9-hex-digit spec typo; the authoritative
/// binary packs to `0x00051041` — see `docs/ci_plus/resource-ids.md`.)
pub const DOWNLOAD: ResourceId = ResourceId(0x0005_1041);

/// StreamInput template — class 128, type 1\*, version 1 (`0x00801ii1`, `ii` =
/// Module ID). Match with [`MODULE_ID_MASK`].
pub const STREAM_INPUT_TEMPLATE: ResourceId = ResourceId(0x0080_1001);
/// BroadcastServiceGateway template — class 129, type 1\* (`0x00811ii1`).
pub const BROADCAST_SERVICE_GATEWAY_TEMPLATE: ResourceId = ResourceId(0x0081_1001);
/// StatusQuery template — class 33, type 1\* (`0x00211ii1`).
pub const STATUS_QUERY_TEMPLATE: ResourceId = ResourceId(0x0021_1001);
/// Event Manager template — class 35, type 1\* (`0x00231ii1`).
pub const EVENT_MANAGER_TEMPLATE: ResourceId = ResourceId(0x0023_1001);
/// Copy Protection template — class 4, type 1\* (`0x00041ii1`).
pub const COPY_PROTECTION_TEMPLATE: ResourceId = ResourceId(0x0004_1001);
/// CA Pipeline template — class 6, type 1\* (`0x00061ii1`).
pub const CA_PIPELINE_TEMPLATE: ResourceId = ResourceId(0x0006_1001);

/// Mask that clears the 6-bit Module ID (`ii`) of a `type = 1*` resource ID —
/// the low 6 bits of the 10-bit `resource_type` field, i.e. bits `[11:6]`
/// (TS 101 699 §8.1). Applying it to a `1*` resource ID yields its template ID.
pub const MODULE_ID_MASK: u32 = 0xFFFF_F03F;

/// Extract the 6-bit Module ID (`ii`) from a `type = 1*` resource ID.
#[must_use]
pub const fn module_id(id: ResourceId) -> u8 {
    ((id.0 >> 6) & 0x3F) as u8
}

/// The DVB CI-extension resource a [`ResourceId`] denotes. `type = 1*` resources
/// are matched after masking out the Module ID; fixed-ID resources match exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CiExtResource {
    /// Resource Manager v2 (`0x00010042`).
    ResourceManagerV2,
    /// Application Information v2 (`0x00020042`).
    ApplicationInfoV2,
    /// Power Manager (`0x00220041`).
    PowerManager,
    /// Application MMI (`0x00410041`).
    ApplicationMmi,
    /// Download resource (`0x00051041`).
    Download,
    /// StreamInput (`0x00801ii1`), carrying its Module ID.
    StreamInput(u8),
    /// BroadcastServiceGateway (`0x00811ii1`), carrying its Module ID.
    BroadcastServiceGateway(u8),
    /// StatusQuery (`0x00211ii1`), carrying its Module ID.
    StatusQuery(u8),
    /// Event Manager (`0x00231ii1`), carrying its Module ID.
    EventManager(u8),
    /// Copy Protection (`0x00041ii1`), carrying its Module ID.
    CopyProtection(u8),
    /// CA Pipeline (`0x00061ii1`), carrying its Module ID.
    CaPipeline(u8),
}

impl CiExtResource {
    /// Diagnostic spec token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ResourceManagerV2 => "resource_manager_v2",
            Self::ApplicationInfoV2 => "application_information_v2",
            Self::PowerManager => "power_manager",
            Self::ApplicationMmi => "application_mmi",
            Self::Download => "download",
            Self::StreamInput(_) => "stream_input",
            Self::BroadcastServiceGateway(_) => "broadcast_service_gateway",
            Self::StatusQuery(_) => "status_query",
            Self::EventManager(_) => "event_manager",
            Self::CopyProtection(_) => "copy_protection",
            Self::CaPipeline(_) => "ca_pipeline",
        }
    }
}

impl core::fmt::Display for CiExtResource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StreamInput(ii)
            | Self::BroadcastServiceGateway(ii)
            | Self::StatusQuery(ii)
            | Self::EventManager(ii)
            | Self::CopyProtection(ii)
            | Self::CaPipeline(ii) => write!(f, "{}(module_id={ii})", self.name()),
            other => f.write_str(other.name()),
        }
    }
}

/// Map a [`ResourceId`] to its DVB CI-extension resource. `type = 1*` resources
/// are matched by masking out the Module ID byte; fixed-ID resources match
/// exactly. Returns `None` for any non-CI-extension resource.
#[must_use]
pub fn classify(id: ResourceId) -> Option<CiExtResource> {
    match id {
        RESOURCE_MANAGER_V2 => return Some(CiExtResource::ResourceManagerV2),
        APPLICATION_INFO_V2 => return Some(CiExtResource::ApplicationInfoV2),
        POWER_MANAGER => return Some(CiExtResource::PowerManager),
        APPLICATION_MMI => return Some(CiExtResource::ApplicationMmi),
        DOWNLOAD => return Some(CiExtResource::Download),
        _ => {}
    }
    let ii = module_id(id);
    match ResourceId(id.0 & MODULE_ID_MASK) {
        STREAM_INPUT_TEMPLATE => Some(CiExtResource::StreamInput(ii)),
        BROADCAST_SERVICE_GATEWAY_TEMPLATE => Some(CiExtResource::BroadcastServiceGateway(ii)),
        STATUS_QUERY_TEMPLATE => Some(CiExtResource::StatusQuery(ii)),
        EVENT_MANAGER_TEMPLATE => Some(CiExtResource::EventManager(ii)),
        COPY_PROTECTION_TEMPLATE => Some(CiExtResource::CopyProtection(ii)),
        CA_PIPELINE_TEMPLATE => Some(CiExtResource::CaPipeline(ii)),
        _ => None,
    }
}

/// A parsed DVB CI-extension APDU, scoped to the resource it arrived on.
///
/// One variant per resource implemented this pass; each wraps that resource's
/// own object enum (which dispatches on the leading `apdu_tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CiExtApdu<'a> {
    /// Resource Manager v2 object (`0x00010042`).
    ResourceManagerV2(resource_manager_v2::ResourceManagerV2Apdu),
    /// Application Information v2 object (`0x00020042`).
    ApplicationInfoV2(application_info_v2::ApplicationInfoV2Apdu<'a>),
    /// Power Manager object (`0x00220041`).
    PowerManager(power_manager::PowerManagerApdu),
    /// Event Manager object (`0x00231ii1`).
    EventManager(event_manager::EventManagerApdu<'a>),
    /// Copy Protection object (`0x00041ii1`).
    CopyProtection(copy_protection::CopyProtectionApdu<'a>),
    /// StreamInput object (`0x00801ii1`).
    StreamInput(stream_input::StreamInputApdu<'a>),
    /// Broadcast Service Gateway object (`0x00811ii1`), including the inherited
    /// Generic Service Gateway calls.
    BroadcastServiceGateway(broadcast_service_gateway::BroadcastServiceGatewayApdu<'a>),
    /// Status Query object (`0x00211ii1`).
    StatusQuery(status_query::StatusQueryApdu<'a>),
    /// Application MMI object (`0x00410041`).
    ApplicationMmi(application_mmi::ApplicationMmiApdu<'a>),
    /// Download (CAM firmware) object (`0x00051041`).
    Download(software_download::DownloadApdu<'a>),
    /// CA Pipeline object (`0x00061ii1`).
    CaPipeline(ca_pipeline::CaPipelineApdu<'a>),
}

impl<'a> CiExtApdu<'a> {
    /// Parse a CI-extension APDU, selecting the resource from `resource_id`
    /// (Module ID masked out for `type = 1*` resources) and then delegating to
    /// that resource's object dispatch on the leading `apdu_tag`.
    ///
    /// Errors with [`Error::UnknownResource`] if `resource_id` is not a
    /// CI-extension resource handled this pass.
    pub fn parse(resource_id: ResourceId, body: &'a [u8]) -> Result<Self> {
        match classify(resource_id) {
            Some(CiExtResource::ResourceManagerV2) => Ok(Self::ResourceManagerV2(
                resource_manager_v2::ResourceManagerV2Apdu::parse(body)?,
            )),
            Some(CiExtResource::ApplicationInfoV2) => Ok(Self::ApplicationInfoV2(
                application_info_v2::ApplicationInfoV2Apdu::parse(body)?,
            )),
            Some(CiExtResource::PowerManager) => Ok(Self::PowerManager(
                power_manager::PowerManagerApdu::parse(body)?,
            )),
            Some(CiExtResource::EventManager(_)) => Ok(Self::EventManager(
                event_manager::EventManagerApdu::parse(body)?,
            )),
            Some(CiExtResource::CopyProtection(_)) => Ok(Self::CopyProtection(
                copy_protection::CopyProtectionApdu::parse(body)?,
            )),
            Some(CiExtResource::StreamInput(_)) => Ok(Self::StreamInput(
                stream_input::StreamInputApdu::parse(body)?,
            )),
            Some(CiExtResource::BroadcastServiceGateway(_)) => Ok(Self::BroadcastServiceGateway(
                broadcast_service_gateway::BroadcastServiceGatewayApdu::parse(body)?,
            )),
            Some(CiExtResource::StatusQuery(_)) => Ok(Self::StatusQuery(
                status_query::StatusQueryApdu::parse(body)?,
            )),
            Some(CiExtResource::ApplicationMmi) => Ok(Self::ApplicationMmi(
                application_mmi::ApplicationMmiApdu::parse(body)?,
            )),
            Some(CiExtResource::Download) => Ok(Self::Download(
                software_download::DownloadApdu::parse(body)?,
            )),
            Some(CiExtResource::CaPipeline(_)) => {
                Ok(Self::CaPipeline(ca_pipeline::CaPipelineApdu::parse(body)?))
            }
            _ => Err(Error::UnknownResource {
                resource_id: resource_id.0,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_fixed_ids() {
        assert_eq!(
            classify(RESOURCE_MANAGER_V2),
            Some(CiExtResource::ResourceManagerV2)
        );
        assert_eq!(
            classify(APPLICATION_INFO_V2),
            Some(CiExtResource::ApplicationInfoV2)
        );
        assert_eq!(classify(POWER_MANAGER), Some(CiExtResource::PowerManager));
        assert_eq!(classify(ResourceId(0xDEAD_BEEF)), None);
        // The EN 50221 v1 Resource Manager (0x00010041) is NOT the v2 ID.
        assert_eq!(classify(ResourceId(0x0001_0041)), None);
    }

    #[test]
    fn classify_masks_module_id() {
        // Copy Protection module_id 3 -> 0x000410C1 (TS 101 699 §6.6.1.1 example).
        let cp3 = ResourceId(0x0004_10C1);
        assert_eq!(classify(cp3), Some(CiExtResource::CopyProtection(3)));
        assert_eq!(module_id(cp3), 3);
        // Event Manager module_id 1 -> 0x00231041, module_id 2 -> 0x00231081.
        assert_eq!(
            classify(ResourceId(0x0023_1041)),
            Some(CiExtResource::EventManager(1))
        );
        assert_eq!(
            classify(ResourceId(0x0023_1081)),
            Some(CiExtResource::EventManager(2))
        );
        // module_id 0 still classifies.
        assert_eq!(
            classify(EVENT_MANAGER_TEMPLATE),
            Some(CiExtResource::EventManager(0))
        );
    }

    #[test]
    fn same_tag_different_resource_routes_differently() {
        // 0x9F8000 means different objects under different resources.
        // Power Manager activation request: tag + len(1) + reserved/state(1).
        let pm_body = [0x9F, 0x80, 0x00, 0x01, 0x00];
        // Copy Protection CP_query: tag + len(3) + CopyProtectionID(3).
        let cp_body = [0x9F, 0x80, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        // Event Manager event request: tag + len(1) + event_type(1).
        let em_body = [0x9F, 0x80, 0x00, 0x01, 0x00];

        let pm = CiExtApdu::parse(POWER_MANAGER, &pm_body).unwrap();
        assert!(matches!(pm, CiExtApdu::PowerManager(_)));

        let cp = CiExtApdu::parse(ResourceId(0x0004_10C1), &cp_body).unwrap();
        assert!(matches!(cp, CiExtApdu::CopyProtection(_)));

        let em = CiExtApdu::parse(ResourceId(0x0023_1041), &em_body).unwrap();
        assert!(matches!(em, CiExtApdu::EventManager(_)));
    }

    #[test]
    fn stream_input_and_bsg_classify_and_mask() {
        // StreamInput type=1*: module_id 1 -> 0x00801041, module_id 5 -> 0x00801141.
        assert_eq!(
            classify(ResourceId(0x0080_1041)),
            Some(CiExtResource::StreamInput(1))
        );
        assert_eq!(
            classify(ResourceId(0x0080_1141)),
            Some(CiExtResource::StreamInput(5))
        );
        assert_eq!(module_id(ResourceId(0x0080_1141)), 5);
        // BroadcastServiceGateway module_id 1 -> 0x00811041 (md example).
        assert_eq!(
            classify(ResourceId(0x0081_1041)),
            Some(CiExtResource::BroadcastServiceGateway(1))
        );
        // module_id 0 templates still classify.
        assert_eq!(
            classify(STREAM_INPUT_TEMPLATE),
            Some(CiExtResource::StreamInput(0))
        );
        assert_eq!(
            classify(BROADCAST_SERVICE_GATEWAY_TEMPLATE),
            Some(CiExtResource::BroadcastServiceGateway(0))
        );
    }

    #[test]
    fn stream_input_9f8000_vs_power_manager_9f8000() {
        // The resource-scoped invariant: 9F8000 means different objects per
        // resource, even with the new resources added.
        // StreamInput 9F8000 = DeliverySystemInfoReq (header-only).
        let si_body = [0x9F, 0x80, 0x00, 0x00];
        let si = CiExtApdu::parse(ResourceId(0x0080_1041), &si_body).unwrap();
        assert!(matches!(
            si,
            CiExtApdu::StreamInput(stream_input::StreamInputApdu::DeliverySystemInfoReq(_))
        ));
        // PowerManager 9F8000 = activation_state_change_request (1-byte body).
        let pm_body = [0x9F, 0x80, 0x00, 0x01, 0x00];
        let pm = CiExtApdu::parse(POWER_MANAGER, &pm_body).unwrap();
        assert!(matches!(pm, CiExtApdu::PowerManager(_)));
        // Same leading tag, different parsed variant => routes by resource.
        assert!(!matches!(si, CiExtApdu::PowerManager(_)));
    }

    #[test]
    fn bsg_dispatch_routes_eit_and_inherited() {
        let bsg = ResourceId(0x0081_1041);
        // 9F8010 EITSectionReq (BSG-specific extension).
        let eit = [
            0x9F, 0x80, 0x10, 0x08, 0x00, 0x4E, 0x00, 0x64, 0x00, 0x00, 0x01, 0x00,
        ];
        let parsed = CiExtApdu::parse(bsg, &eit).unwrap();
        assert!(matches!(
            parsed,
            CiExtApdu::BroadcastServiceGateway(
                broadcast_service_gateway::BroadcastServiceGatewayApdu::EitSectionReq(_)
            )
        ));
        // 9F8000 inherited Generic Service Gateway ServiceListReq.
        let slr = [0x9F, 0x80, 0x00, 0x00];
        let parsed = CiExtApdu::parse(bsg, &slr).unwrap();
        assert!(matches!(
            parsed,
            CiExtApdu::BroadcastServiceGateway(
                broadcast_service_gateway::BroadcastServiceGatewayApdu::ServiceGateway(_)
            )
        ));
    }

    #[test]
    fn new_type1_ids_classify_and_mask() {
        // StatusQuery type=1*: module_id 1 -> 0x00211041, module_id 2 -> 0x00211081.
        assert_eq!(
            classify(ResourceId(0x0021_1041)),
            Some(CiExtResource::StatusQuery(1))
        );
        assert_eq!(
            classify(ResourceId(0x0021_1081)),
            Some(CiExtResource::StatusQuery(2))
        );
        assert_eq!(module_id(ResourceId(0x0021_1081)), 2);
        // CA Pipeline type=1*: 0x00061041 (module 1), 0x00061081 (module 2) — md examples.
        assert_eq!(
            classify(ResourceId(0x0006_1041)),
            Some(CiExtResource::CaPipeline(1))
        );
        assert_eq!(
            classify(ResourceId(0x0006_1081)),
            Some(CiExtResource::CaPipeline(2))
        );
        // module_id 0 templates still classify.
        assert_eq!(
            classify(STATUS_QUERY_TEMPLATE),
            Some(CiExtResource::StatusQuery(0))
        );
        assert_eq!(
            classify(CA_PIPELINE_TEMPLATE),
            Some(CiExtResource::CaPipeline(0))
        );
        // ApplicationMMI / Download are fixed IDs.
        assert_eq!(
            classify(APPLICATION_MMI),
            Some(CiExtResource::ApplicationMmi)
        );
        assert_eq!(classify(DOWNLOAD), Some(CiExtResource::Download));
    }

    #[test]
    fn tag_9f8000_routes_per_resource_across_all_resources() {
        // The resource-scoped invariant with the full resource set present:
        // 9F8000 means a different object under each resource.
        let body = [0x9F, 0x80, 0x00, 0x00]; // header-only / empty body

        // StatusQuery 9F8000 = StatusQueryReq (needs a 4-byte StatusItem body).
        let sq_body = [0x9F, 0x80, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01];
        let sq = CiExtApdu::parse(ResourceId(0x0021_1041), &sq_body).unwrap();
        assert!(matches!(
            sq,
            CiExtApdu::StatusQuery(status_query::StatusQueryApdu::StatusQueryReq(_))
        ));

        // ApplicationMMI 9F8000 = RequestStart (2-byte minimum body).
        let mmi_body = [0x9F, 0x80, 0x00, 0x02, 0x00, 0x00];
        let mmi = CiExtApdu::parse(APPLICATION_MMI, &mmi_body).unwrap();
        assert!(matches!(
            mmi,
            CiExtApdu::ApplicationMmi(application_mmi::ApplicationMmiApdu::RequestStart(_))
        ));

        // Download 9F8000 = download_enq (opaque body).
        let dl = CiExtApdu::parse(DOWNLOAD, &body).unwrap();
        assert!(matches!(
            dl,
            CiExtApdu::Download(software_download::DownloadApdu::DownloadEnquiry(_))
        ));

        // CA Pipeline 9F8000 = CAPipelineRequest (opaque body).
        let cap = CiExtApdu::parse(ResourceId(0x0006_1041), &body).unwrap();
        assert!(matches!(
            cap,
            CiExtApdu::CaPipeline(ca_pipeline::CaPipelineApdu::Request(_))
        ));

        // StreamInput 9F8000 = DeliverySystemInfoReq — an *earlier* resource's 9F8000.
        let si = CiExtApdu::parse(ResourceId(0x0080_1041), &body).unwrap();
        assert!(matches!(
            si,
            CiExtApdu::StreamInput(stream_input::StreamInputApdu::DeliverySystemInfoReq(_))
        ));

        // All four new + the earlier one are distinct CiExtApdu variants.
        assert!(!matches!(sq, CiExtApdu::ApplicationMmi(_)));
        assert!(!matches!(dl, CiExtApdu::CaPipeline(_)));
        assert!(!matches!(cap, CiExtApdu::Download(_)));
    }

    #[test]
    fn unknown_resource_errors() {
        let body = [0x9F, 0x80, 0x00, 0x00];
        assert!(matches!(
            CiExtApdu::parse(ResourceId(0x1234_5678), &body),
            Err(Error::UnknownResource { .. })
        ));
    }

    #[test]
    fn display_carries_module_id() {
        use alloc::format;
        assert_eq!(
            format!("{}", CiExtResource::CopyProtection(3)),
            "copy_protection(module_id=3)"
        );
        assert_eq!(format!("{}", CiExtResource::PowerManager), "power_manager");
    }
}
