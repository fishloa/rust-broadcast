#![cfg(feature = "std")]

use dvb_mabr::{Error, MulticastGatewayConfiguration, MulticastServerConfiguration};

fn server_with(from: &str, to: &str) -> String {
    let xml = test_fixtures::read_to_string("mabr/annex-c1-server-config.xml");
    assert!(xml.contains(from), "fixture lost {from:?}");
    xml.replacen(from, to, 1)
}

#[test]
fn a_component_identifier_without_xsi_type_is_a_missing_component_type() {
    let xml = server_with("xsi:type=\"DASHComponentIdentifierType\" ", "");
    assert_eq!(
        MulticastServerConfiguration::parse_str(&xml).unwrap_err(),
        Error::MissingComponentType
    );
}

#[test]
fn an_unrecognised_xsi_type_is_an_unknown_component_type_naming_it() {
    let xml = server_with("HLSComponentIdentifierType", "BogusComponentIdentifierType");
    assert_eq!(
        MulticastServerConfiguration::parse_str(&xml).unwrap_err(),
        Error::UnknownComponentType("BogusComponentIdentifierType".to_string())
    );
}

#[test]
fn a_component_identifier_without_its_manifest_ref_is_a_missing_attribute() {
    let xml = server_with("manifestIdRef=\"bbc-one-scotland_mpd\"", "");
    assert_eq!(
        MulticastServerConfiguration::parse_str(&xml).unwrap_err(),
        Error::MissingAttribute {
            element: "ServiceComponentIdentifier",
            attr: "manifestIdRef",
        }
    );
}

#[test]
fn a_transport_session_without_a_bit_rate_is_a_missing_element() {
    let xml = test_fixtures::read_to_string("mabr/annex-c3-gateway-config.xml");
    let bit_rate = "<BitRate average=\"150000\" maximum=\"150000\"/>";
    assert!(xml.contains(bit_rate));
    let xml = xml.replacen(bit_rate, "", 1);
    assert_eq!(
        MulticastGatewayConfiguration::parse_str(&xml).unwrap_err(),
        Error::MissingElement {
            parent: "MulticastTransportSession",
            child: "BitRate",
        }
    );
}
