//! Audit-22 #3046: each `child.is(X) && field.is_none()` guard in the clause
//! 10.2.3/10.2.5 parsers and the `xsi:type` attribute selector must keep the
//! first singleton child, ignore unknown (or known-but-different) siblings in
//! any order, and select a component variant only from a real `xsi:type`.
#![cfg(feature = "std")]

use dvb_mabr::{
    Error, MulticastGatewayConfiguration, MulticastServerConfiguration, ServiceComponentIdentifier,
};

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
fn server(body: &str) -> String {
    format!(
        r#"<MulticastServerConfiguration xmlns="{NS}" xmlns:xsi="{XSI}" schemaVersion="2">{body}</MulticastServerConfiguration>"#
    )
}

fn gateway(body: &str) -> String {
    format!(
        r#"<MulticastGatewayConfiguration xmlns="{NS}" xmlns:xsi="{XSI}" schemaVersion="2">{body}</MulticastGatewayConfiguration>"#
    )
}

fn session(inner: &str) -> String {
    format!(r#"<MulticastSession serviceIdentifier="urn:example:service">{inner}</MulticastSession>"#)
}

fn transport_session(inner: &str) -> String {
    format!(r#"<MulticastTransportSession id="ts1" sessionIdleTimeout="1000">{inner}</MulticastTransportSession>"#)
}

fn transport_protocol(id: &str) -> String {
    format!(r#"<TransportProtocol protocolIdentifier="{id}" protocolVersion="1"/>"#)
}

fn parse_server(xml: &str) -> MulticastServerConfiguration {
    MulticastServerConfiguration::parse_str(xml).expect("parse server document")
}
// --- clause 10.2.3.11: ForwardErrorCorrectionParameters --------------------

#[test]
fn duplicate_fec_children_keep_the_first_value() {
    let fec = r#"<ForwardErrorCorrectionParameters>
      <SchemeIdentifier>urn:ietf:rmt:fec:encoding:1</SchemeIdentifier>
      <SchemeIdentifier>urn:ietf:rmt:fec:encoding:6</SchemeIdentifier>
      <OverheadPercentage>20</OverheadPercentage>
      <OverheadPercentage>30</OverheadPercentage>
    </ForwardErrorCorrectionParameters>"#;
    let xml = server(&session(&transport_session(&format!(
        "{}{fec}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol")
    ))));
    let parsed = parse_server(&xml);
    let fec = &parsed.sessions[0].transport_sessions[0].fec_params[0];
    assert_eq!(fec.scheme_identifier, "urn:ietf:rmt:fec:encoding:1");
    assert_eq!(fec.overhead_percentage, 20);
}

#[test]
fn a_known_child_before_the_fec_scheme_is_not_read_as_the_scheme() {
    let fec = r#"<ForwardErrorCorrectionParameters>
      <OverheadPercentage>20</OverheadPercentage>
      <SchemeIdentifier>urn:ietf:rmt:fec:encoding:1</SchemeIdentifier>
    </ForwardErrorCorrectionParameters>"#;
    let xml = server(&session(&transport_session(&format!(
        "{}{fec}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol")
    ))));
    let parsed = parse_server(&xml);
    let fec = &parsed.sessions[0].transport_sessions[0].fec_params[0];
    assert_eq!(fec.scheme_identifier, "urn:ietf:rmt:fec:encoding:1");
    assert_eq!(fec.overhead_percentage, 20);
}

#[test]
fn an_unknown_child_before_the_fec_scheme_is_skipped() {
    let body = |unknown: &str| {
        format!(
            r#"<ForwardErrorCorrectionParameters>{unknown}
      <SchemeIdentifier>urn:ietf:rmt:fec:encoding:1</SchemeIdentifier>
      <OverheadPercentage>20</OverheadPercentage>
    </ForwardErrorCorrectionParameters>"#
        )
    };
    let with = parse_server(&server(&session(&transport_session(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol"),
        body("<Unknown>ignored</Unknown>")
    )))));
    let without = parse_server(&server(&session(&transport_session(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol"),
        body("")
    )))));
    assert_eq!(with, without);
}

// --- clause 10.2.3.9: EndpointAddress --------------------------------------

fn endpoint(children: &str) -> String {
    format!("<EndpointAddress>{children}</EndpointAddress>")
}

#[test]
fn duplicate_endpoint_children_keep_the_first_value() {
    let ep = endpoint(
        r#"<NetworkSourceAddress>10.0.0.1</NetworkSourceAddress>
           <NetworkSourceAddress>10.0.0.2</NetworkSourceAddress>
           <NetworkDestinationGroupAddress>239.1.1.1</NetworkDestinationGroupAddress>
           <NetworkDestinationGroupAddress>239.1.1.2</NetworkDestinationGroupAddress>
           <TransportDestinationPort>5000</TransportDestinationPort>
           <TransportDestinationPort>6000</TransportDestinationPort>
           <MediaTransportSessionIdentifier>7</MediaTransportSessionIdentifier>
           <MediaTransportSessionIdentifier>8</MediaTransportSessionIdentifier>"#,
    );
    let xml = server(&session(&transport_session(&format!(
        "{}{ep}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol")
    ))));
    let parsed = parse_server(&xml);
    let ep = &parsed.sessions[0].transport_sessions[0].endpoints[0];
    assert_eq!(ep.source.as_deref(), Some("10.0.0.1"));
    assert_eq!(ep.group, "239.1.1.1");
    assert_eq!(ep.port, 5000);
    assert_eq!(ep.transport_session_id, Some(7));
}

#[test]
fn a_known_child_before_the_endpoint_group_is_not_read_as_the_group() {
    let ep = endpoint(
        r#"<TransportDestinationPort>5000</TransportDestinationPort>
           <NetworkDestinationGroupAddress>239.1.1.1</NetworkDestinationGroupAddress>"#,
    );
    let xml = server(&session(&transport_session(&format!(
        "{}{ep}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol")
    ))));
    let parsed = parse_server(&xml);
    let ep = &parsed.sessions[0].transport_sessions[0].endpoints[0];
    assert_eq!(ep.group, "239.1.1.1");
    assert_eq!(ep.port, 5000);
}

#[test]
fn an_unknown_child_before_the_endpoint_group_is_skipped() {
    let body = |unknown: &str| {
        endpoint(&format!(
            r#"{unknown}
           <NetworkDestinationGroupAddress>239.1.1.1</NetworkDestinationGroupAddress>
           <TransportDestinationPort>5000</TransportDestinationPort>"#
        ))
    };
    let with = parse_server(&server(&session(&transport_session(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol"),
        body("<Unknown>ignored</Unknown>")
    )))));
    let without = parse_server(&server(&session(&transport_session(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:protocol"),
        body("")
    )))));
    assert_eq!(with, without);
}

// --- clause 10.2.3.1: MulticastTransportSession ----------------------------

#[test]
fn duplicate_transport_protocol_keeps_the_first_value() {
    let xml = server(&session(&transport_session(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:first"),
        transport_protocol("urn:example:second")
    ))));
    let parsed = parse_server(&xml);
    let ts = &parsed.sessions[0].transport_sessions[0];
    assert_eq!(ts.transport_protocol.protocol_identifier, "urn:example:first");
}

#[test]
fn a_known_child_before_the_transport_protocol_is_not_read_as_it() {
    let xml = server(&session(&transport_session(&format!(
        "<BitRate maximum=\"1000\"/>{}{}",
        transport_protocol("urn:example:protocol"),
        transport_protocol("urn:example:other")
    ))));
    let parsed = parse_server(&xml);
    let ts = &parsed.sessions[0].transport_sessions[0];
    assert_eq!(ts.transport_protocol.protocol_identifier, "urn:example:protocol");
}
// --- clause 10.2.5: MulticastGatewayConfigurationTransportSession -----------

fn gateway_transport(inner: &str) -> String {
    format!(
        r#"<MulticastGatewayConfigurationTransportSession>{inner}</MulticastGatewayConfigurationTransportSession>"#
    )
}

#[test]
fn duplicate_gateway_transport_protocol_keeps_the_first_value() {
    let xml = gateway(&gateway_transport(&format!(
        "{}{}<BitRate maximum=\"1000\"/>",
        transport_protocol("urn:example:first"),
        transport_protocol("urn:example:second")
    )));
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse gateway document");
    let ts = &parsed.gateway_config_transport_sessions[0];
    assert_eq!(ts.transport_protocol.protocol_identifier, "urn:example:first");
}

#[test]
fn duplicate_gateway_object_carousel_keeps_the_first_value() {
    let xml = gateway(&gateway_transport(&format!(
        "{}{}<BitRate maximum=\"1000\"/><ObjectCarousel aggregateTransportSize=\"1\"/><ObjectCarousel aggregateTransportSize=\"2\"/>",
        transport_protocol("urn:example:protocol"),
        ""
    )));
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse gateway document");
    let ts = &parsed.gateway_config_transport_sessions[0];
    assert_eq!(
        ts.object_carousel
            .as_ref()
            .expect("the first carousel is kept")
            .aggregate_transport_size,
        Some(1)
    );
}

#[test]
fn a_known_child_before_the_gateway_transport_protocol_is_not_read_as_it() {
    let xml = gateway(&gateway_transport(&format!(
        "<BitRate maximum=\"1000\"/>{}{}",
        transport_protocol("urn:example:protocol"),
        transport_protocol("urn:example:other")
    )));
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse gateway document");
    let ts = &parsed.gateway_config_transport_sessions[0];
    assert_eq!(ts.transport_protocol.protocol_identifier, "urn:example:protocol");
}

#[test]
fn an_unknown_child_before_the_gateway_transport_protocol_is_skipped() {
    let body = |unknown: &str| {
        format!(
            "{unknown}{}<BitRate maximum=\"1000\"/>",
            transport_protocol("urn:example:protocol")
        )
    };
    let with = MulticastGatewayConfiguration::parse_str(&gateway(&gateway_transport(&body(
        "<Unknown>ignored</Unknown>",
    ))))
    .expect("parse");
    let without =
        MulticastGatewayConfiguration::parse_str(&gateway(&gateway_transport(&body(""))))
            .expect("parse");
    assert_eq!(with, without);
}

// --- clause 10.2.4: ServiceComponentIdentifier/@xsi:type -------------------

#[test]
fn a_component_identifier_with_a_plain_type_attribute_is_still_untyped() {
    let xml = server(&session(&transport_session(&format!(
        r#"{tp}<BitRate maximum="1000"/><ServiceComponentIdentifier type="GenericComponentIdentifierType" manifestIdRef="m" componentIdentifier="c"/>"#,
        tp = transport_protocol("urn:example:protocol")
    ))));
    match MulticastServerConfiguration::parse_str(&xml) {
        Err(Error::MissingComponentType) => {}
        other => panic!("a plain (un-namespaced) type attribute must not select a variant: {other:?}"),
    }
}

#[test]
fn a_component_identifier_with_only_another_xsi_attribute_is_still_untyped() {
    let xml = server(&session(&transport_session(&format!(
        r#"{tp}<BitRate maximum="1000"/><ServiceComponentIdentifier xsi:other="GenericComponentIdentifierType" manifestIdRef="m" componentIdentifier="c"/>"#,
        tp = transport_protocol("urn:example:protocol")
    ))));
    match MulticastServerConfiguration::parse_str(&xml) {
        Err(Error::MissingComponentType) => {}
        other => panic!("only xsi:type selects a variant, got: {other:?}"),
    }
}

#[test]
fn a_real_xsi_type_still_selects_its_variant() {
    let xml = server(&session(&transport_session(&format!(
        r#"{tp}<BitRate maximum="1000"/><ServiceComponentIdentifier xsi:type="GenericComponentIdentifierType" manifestIdRef="m" componentIdentifier="c"/>"#,
        tp = transport_protocol("urn:example:protocol")
    ))));
    let parsed = parse_server(&xml);
    let ids = &parsed.sessions[0].transport_sessions[0].service_component_ids;
    assert!(matches!(
        &ids[0],
        ServiceComponentIdentifier::Generic { manifest_id_ref, component_id }
            if manifest_id_ref == "m" && component_id == "c"
    ));
}
