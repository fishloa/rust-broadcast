//! Audit-22 #3044: the root-element identity check must compare both the
//! local name and a baseline namespace, so a gateway document cannot be
//! parsed as a server configuration (or vice versa) and a right-name/
//! foreign-namespace root is refused.
#![cfg(feature = "std")]

use dvb_mabr::{Error, MulticastGatewayConfiguration, MulticastServerConfiguration};

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";

fn fixture(name: &str) -> String {
    test_fixtures::read_to_string(&format!("mabr/{name}"))
}

fn assert_unexpected_root<T: std::fmt::Debug>(result: Result<T, Error>, local: &str) {
    match result {
        Err(Error::UnexpectedRoot(root)) => assert!(
            root.contains(local),
            "UnexpectedRoot must name the actual local name {local:?}, got {root:?}"
        ),
        other => panic!("expected UnexpectedRoot({local}), got {other:?}"),
    }
}

#[test]
fn a_gateway_document_is_not_a_server_configuration() {
    let xml = fixture("annex-c2-gateway-bootstrap.xml");
    assert_unexpected_root(
        MulticastServerConfiguration::parse_str(&xml),
        "MulticastGatewayConfiguration",
    );
}

#[test]
fn a_server_document_is_not_a_gateway_configuration() {
    let xml = fixture("annex-c1-server-config.xml");
    assert_unexpected_root(
        MulticastGatewayConfiguration::parse_str(&xml),
        "MulticastServerConfiguration",
    );
}

#[test]
fn a_root_with_the_right_name_but_a_foreign_namespace_is_rejected() {
    assert_unexpected_root(
        MulticastServerConfiguration::parse_str(
            r#"<MulticastServerConfiguration xmlns="urn:example:foreign" schemaVersion="2"/>"#,
        ),
        "MulticastServerConfiguration",
    );
    assert_unexpected_root(
        MulticastGatewayConfiguration::parse_str(
            r#"<MulticastGatewayConfiguration xmlns="urn:example:foreign" schemaVersion="2"/>"#,
        ),
        "MulticastGatewayConfiguration",
    );
}

#[test]
fn a_root_with_the_right_namespace_but_the_wrong_name_is_rejected() {
    assert_unexpected_root(
        MulticastServerConfiguration::parse_str(&format!(
            r#"<MulticastGatewayConfiguration xmlns="{NS}" schemaVersion="2"/>"#
        )),
        "MulticastGatewayConfiguration",
    );
    assert_unexpected_root(
        MulticastGatewayConfiguration::parse_str(&format!(
            r#"<MulticastServerConfiguration xmlns="{NS}" schemaVersion="2"/>"#
        )),
        "MulticastServerConfiguration",
    );
}
