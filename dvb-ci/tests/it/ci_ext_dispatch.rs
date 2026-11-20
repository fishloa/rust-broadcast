//! `CiExtApdu::parse` dispatch: every CI-extension resource must route to its
//! own object enum (ETSI TS 101 699 §8, Table 87).

use dvb_ci::ci_ext::{
    APPLICATION_INFO_V2, APPLICATION_MMI, BROADCAST_SERVICE_GATEWAY_TEMPLATE, CA_PIPELINE_TEMPLATE,
    COPY_PROTECTION_TEMPLATE, CiExtApdu, DOWNLOAD, EVENT_MANAGER_TEMPLATE, POWER_MANAGER,
    RESOURCE_MANAGER_V2, STATUS_QUERY_TEMPLATE, STREAM_INPUT_TEMPLATE, classify,
};
use dvb_ci::resource::ResourceId;

fn variant_name(apdu: &CiExtApdu<'_>) -> &'static str {
    match apdu {
        CiExtApdu::ResourceManagerV2(_) => "resource_manager_v2",
        CiExtApdu::ApplicationInfoV2(_) => "application_information_v2",
        CiExtApdu::PowerManager(_) => "power_manager",
        CiExtApdu::EventManager(_) => "event_manager",
        CiExtApdu::CopyProtection(_) => "copy_protection",
        CiExtApdu::StreamInput(_) => "stream_input",
        CiExtApdu::BroadcastServiceGateway(_) => "broadcast_service_gateway",
        CiExtApdu::StatusQuery(_) => "status_query",
        CiExtApdu::ApplicationMmi(_) => "application_mmi",
        CiExtApdu::Download(_) => "download",
        CiExtApdu::CaPipeline(_) => "ca_pipeline",
        _ => "unknown",
    }
}

#[test]
fn every_resource_routes_to_its_own_dispatch_arm() {
    let cases: &[(ResourceId, &[u8], &str)] = &[
        (
            RESOURCE_MANAGER_V2,
            &[0x9F, 0x80, 0x10, 0x00],
            "resource_manager_v2",
        ),
        (
            APPLICATION_INFO_V2,
            &[0x9F, 0x80, 0x20, 0x00],
            "application_information_v2",
        ),
        (
            POWER_MANAGER,
            &[0x9F, 0x80, 0x00, 0x01, 0x00],
            "power_manager",
        ),
        (
            EVENT_MANAGER_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x01, 0x00],
            "event_manager",
        ),
        (
            COPY_PROTECTION_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x03, 0xAA, 0xBB, 0xCC],
            "copy_protection",
        ),
        (
            STREAM_INPUT_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x00],
            "stream_input",
        ),
        (
            BROADCAST_SERVICE_GATEWAY_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x00],
            "broadcast_service_gateway",
        ),
        (
            STATUS_QUERY_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01],
            "status_query",
        ),
        (
            APPLICATION_MMI,
            &[0x9F, 0x80, 0x00, 0x02, 0x00, 0x00],
            "application_mmi",
        ),
        (DOWNLOAD, &[0x9F, 0x80, 0x00, 0x00], "download"),
        (
            CA_PIPELINE_TEMPLATE,
            &[0x9F, 0x80, 0x00, 0x00],
            "ca_pipeline",
        ),
    ];
    let mut seen = Vec::new();
    for &(resource_id, body, label) in cases {
        let parsed = CiExtApdu::parse(resource_id, body).expect("parses");
        let classified = classify(resource_id).expect("classifies");
        assert_eq!(classified.name(), label, "{resource_id:?}");
        assert_eq!(variant_name(&parsed), label, "{resource_id:?}");
        seen.push(label);
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        cases.len(),
        "two resources share a dispatch arm"
    );
}
