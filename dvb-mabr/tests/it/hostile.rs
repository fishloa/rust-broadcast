//! Audit-22 #3042: the crate's "DOCTYPE refused" policy must hold at every
//! event read — inside a skipped subtree, between children, inside modelled
//! text, and after a semantic error (`drain`) — and a nested element inside a
//! text-only element is skipped in depth order instead of being parsed as the
//! outer text.
#![cfg(feature = "std")]

use dvb_mabr::error::StructureError;
use dvb_mabr::{Error, MulticastGatewayConfiguration};

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";

fn gateway_with(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS}" schemaVersion="2">{body}</MulticastGatewayConfiguration>"#
    )
}

fn assert_doctype(xml: &str, what: &str) {
    match MulticastGatewayConfiguration::parse_str(xml) {
        Err(Error::InvalidStructure(StructureError::Doctype)) => {}
        other => panic!("{what}: expected the DOCTYPE refusal, got {other:?}"),
    }
}

/// A DOCTYPE hidden inside an unrecognised child element (`skip_content`).
#[test]
fn a_doctype_inside_an_unknown_child_element_is_refused() {
    assert_doctype(
        &gateway_with(r#"<Unknown><!DOCTYPE x [<!ENTITY e "v">]></Unknown>"#),
        "DOCTYPE inside a skipped subtree",
    );
}

/// A DOCTYPE between two children of an element (`next_tag`).
#[test]
fn a_doctype_between_children_is_refused() {
    assert_doctype(
        &gateway_with(r#"<!DOCTYPE x><MulticastSession serviceIdentifier="s"/>"#),
        "DOCTYPE between children",
    );
}

/// A DOCTYPE inside a text-only element (`text`).
#[test]
fn a_doctype_inside_a_text_element_is_refused() {
    let xml = gateway_with(
        r#"<MulticastSession serviceIdentifier="s"><PresentationManifestLocator manifestId="m" contentType="c"><!DOCTYPE x>https://example.com</PresentationManifestLocator></MulticastSession>"#,
    );
    assert_doctype(&xml, "DOCTYPE inside modelled text");
}

/// A DOCTYPE later in the document, after a semantic error has already been
/// found: `drain` must still surface it (`drain`'s `DocType` arm).
#[test]
fn a_doctype_after_a_semantic_error_is_refused_by_drain() {
    let xml = gateway_with(r#"<!DOCTYPE x>"#);
    let xml = xml.replacen("schemaVersion=\"2\"", "schemaVersion=\"two\"", 1);
    assert_doctype(&xml, "DOCTYPE surfaced by drain");
}

#[test]
fn a_nested_element_inside_a_text_element_does_not_absorb_its_text() {
    let xml = gateway_with(
        r#"<MulticastSession serviceIdentifier="s">
  <PresentationManifestLocator manifestId="m1" contentType="c">a<x>b</x>c</PresentationManifestLocator>
  <PresentationManifestLocator manifestId="m2" contentType="c">second</PresentationManifestLocator>
</MulticastSession>"#,
    );
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse");
    let locators = &parsed.sessions[0].manifest_locators;
    assert_eq!(
        locators.len(),
        2,
        "the nested element must not end the session"
    );
    assert_eq!(locators[0].locator, "ac");
    assert_eq!(locators[1].locator, "second");
}
