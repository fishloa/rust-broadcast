//! Structural round-trip test: `parse_str -> to_xml -> parse_str` must yield
//! an equal document for all three committed fixtures (see
//! `fixtures/PROVENANCE.md`). Byte-identical XML is explicitly NOT required
//! for a text/markup format (attribute order, whitespace, and empty-element
//! spelling are not preserved) — see the crate-root doc comment.

use std::fs;
use std::path::PathBuf;

use dvb_mabr::{MulticastGatewayConfiguration, MulticastServerConfiguration};

fn fixture_path(name: &str) -> PathBuf {
    // Fixtures live in the workspace-shared `fixtures/dvb-mabr/` directory,
    // not inside this crate — per project convention.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("dvb-mabr")
        .join(name)
}

fn read_fixture(name: &str) -> String {
    let path = fixture_path(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {path:?}: {e}"))
}

#[test]
fn server_config_round_trips() {
    let xml = read_fixture("annex-c1-server-config.xml");
    let parsed = MulticastServerConfiguration::parse_str(&xml).expect("parse fixture 1");

    // Sanity: the fixture actually exercises the model, so a stubbed-out
    // parser (e.g. one that returns empty Vecs) would fail these asserts.
    assert_eq!(parsed.schema_version, 2);
    assert_eq!(parsed.gateway_config_transport_sessions.len(), 2);
    assert_eq!(parsed.sessions.len(), 2);
    assert_eq!(parsed.macros.len(), 2);
    assert_eq!(parsed.sessions[0].transport_sessions.len(), 2);
    assert!(
        !parsed.sessions[0].transport_sessions[0]
            .fec_params
            .is_empty()
    );
    assert!(
        parsed.sessions[0].transport_sessions[0]
            .unicast_repair
            .is_some()
    );
    assert!(
        parsed.sessions[0].transport_sessions[0]
            .object_carousel
            .is_some()
    );
    assert!(parsed.reporting.is_some());

    let xml2 = parsed.to_xml();
    let reparsed =
        MulticastServerConfiguration::parse_str(&xml2).expect("parse serialized fixture 1");
    assert_eq!(parsed, reparsed, "round-trip changed the parsed document");

    // Mutation-bites: changing a field must change the serialized output
    // (kills a raw-passthrough/echo serializer).
    let mut mutated = parsed.clone();
    mutated.schema_version = 99;
    assert_ne!(mutated.to_xml(), xml2);
}

#[test]
fn gateway_bootstrap_round_trips() {
    let xml = read_fixture("annex-c2-gateway-bootstrap.xml");
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse fixture 2");

    assert_eq!(parsed.schema_version, 2);
    assert_eq!(parsed.gateway_config_transport_sessions.len(), 1);
    assert!(
        parsed.sessions.is_empty(),
        "bootstrap document carries no MulticastSession"
    );
    let ts = &parsed.gateway_config_transport_sessions[0];
    assert!(!ts.tags.is_empty());
    assert!(!ts.fec_params.is_empty());
    assert!(ts.object_carousel.is_some());

    let xml2 = parsed.to_xml();
    let reparsed =
        MulticastGatewayConfiguration::parse_str(&xml2).expect("parse serialized fixture 2");
    assert_eq!(parsed, reparsed, "round-trip changed the parsed document");

    let mut mutated = parsed.clone();
    mutated.gateway_config_transport_sessions[0].session_idle_timeout += 1;
    assert_ne!(mutated.to_xml(), xml2);
}

#[test]
fn full_gateway_config_round_trips() {
    let xml = read_fixture("annex-c3-gateway-config.xml");
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse fixture 3");

    assert_eq!(parsed.sessions.len(), 2);
    assert!(parsed.valid_until.is_some());
    // The empty-locator / mandatory-contentPlaybackPathPattern case (clause
    // 10.2.2.2) must round-trip too.
    let alpha = &parsed.sessions[0];
    assert_eq!(alpha.manifest_locators[0].locator, "");
    assert!(
        alpha.manifest_locators[0]
            .content_playback_path_pattern
            .is_some()
    );
    // The gateway-referencing object carousel's extra reference attributes.
    let carousel = parsed.gateway_config_transport_sessions[0]
        .object_carousel
        .as_ref()
        .expect("object carousel");
    assert_eq!(carousel.presentation_manifests.len(), 2);
    assert!(carousel.presentation_manifests[0].service_id_ref.is_some());
    assert!(
        carousel.presentation_manifests[1]
            .transport_session_id_ref
            .is_some()
    );

    let xml2 = parsed.to_xml();
    let reparsed =
        MulticastGatewayConfiguration::parse_str(&xml2).expect("parse serialized fixture 3");
    assert_eq!(parsed, reparsed, "round-trip changed the parsed document");

    let mut mutated = parsed.clone();
    mutated.sessions[0].service_identifier = "urn:example:service:mutated".into();
    assert_ne!(mutated.to_xml(), xml2);
}
