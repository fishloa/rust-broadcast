//! XML escaping and malformed-input gate for the `quick-xml`-backed codec:
//! all five XML specials (`& < > " '`) in attribute values and element text are
//! escaped on output (exact text asserted) and re-parse to the original
//! strings; malformed documents are structured errors, never panics.
#![cfg(feature = "std")]

use dvb_mabr::{Error, MulticastGatewayConfiguration};

/// All five XML specials in one string.
const SPECIALS: &str = "a&b<c>d\"e'f";
/// `SPECIALS` as quick-xml escapes it.
const SPECIALS_ESCAPED: &str = "a&amp;b&lt;c&gt;d&quot;e&apos;f";

const NS: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";

fn gateway_doc(service_identifier: &str, locator: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration schemaVersion="2" xmlns="{NS}">
  <MulticastSession serviceIdentifier="{service_identifier}">
    <PresentationManifestLocator manifestId="m1" contentType="application/dash+xml">{locator}</PresentationManifestLocator>
  </MulticastSession>
</MulticastGatewayConfiguration>"#
    )
}

#[test]
fn specials_in_attributes_and_text_are_escaped_and_round_trip() {
    let xml = gateway_doc(SPECIALS_ESCAPED, SPECIALS_ESCAPED);
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse");
    let session = &parsed.sessions[0];
    assert_eq!(session.service_identifier, SPECIALS);
    assert_eq!(session.manifest_locators[0].locator, SPECIALS);

    let out = parsed.to_xml();
    assert!(
        out.contains(&format!("serviceIdentifier=\"{SPECIALS_ESCAPED}\"")),
        "attribute must be escaped on the wire:\n{out}"
    );
    assert!(
        out.contains(&format!(
            ">{SPECIALS_ESCAPED}</PresentationManifestLocator>"
        )),
        "text must be escaped on the wire:\n{out}"
    );

    let reparsed = MulticastGatewayConfiguration::parse_str(&out).expect("re-parse");
    assert_eq!(reparsed, parsed);
    assert_eq!(reparsed.sessions[0].service_identifier, SPECIALS);
    assert_eq!(reparsed.sessions[0].manifest_locators[0].locator, SPECIALS);
}

#[test]
fn numeric_character_references_and_cdata_decode() {
    let xml = gateway_doc("a&#38;b&#x3C;c", "<![CDATA[https://x/?a=1&b=2]]>");
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse");
    assert_eq!(parsed.sessions[0].service_identifier, "a&b<c");
    assert_eq!(
        parsed.sessions[0].manifest_locators[0].locator,
        "https://x/?a=1&b=2"
    );
}

/// A comment between two text runs must not truncate the value (MABR-W4).
#[test]
fn comment_inside_text_does_not_truncate() {
    let xml = gateway_doc("s", "a<!-- x -->b");
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse");
    assert_eq!(parsed.sessions[0].manifest_locators[0].locator, "ab");
}

/// Hostile input: a long run of entity references decodes without blowing up.
#[test]
fn twenty_thousand_entities_decode() {
    let xml = gateway_doc("s", &"&amp;".repeat(20_000));
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parse");
    assert_eq!(
        parsed.sessions[0].manifest_locators[0].locator,
        "&".repeat(20_000)
    );
}

fn assert_xml_parse_error(xml: &str, what: &str) {
    match MulticastGatewayConfiguration::parse_str(xml) {
        Err(Error::XmlParse(_)) => {}
        other => panic!("{what}: expected Error::XmlParse, got {other:?}"),
    }
}

#[test]
fn malformed_documents_are_structured_errors() {
    let good = gateway_doc("s", "u");
    // (Truncation at every offset is asserted in tests/truncation.rs.)
    assert_xml_parse_error("", "empty input");
    assert_xml_parse_error("not xml", "plain text");
    assert_xml_parse_error(&good[..good.len() - 10], "truncated close tag");
    assert_xml_parse_error(
        &gateway_doc("a&nope;b", "u"),
        "undefined entity in attribute",
    );
    assert_xml_parse_error(&gateway_doc("s", "a&nope;b"), "undefined entity in text");
    assert_xml_parse_error(
        &good.replace("</MulticastSession>", "</Mismatch>"),
        "mismatched end tag",
    );
    assert_xml_parse_error(
        &good.replace(
            "MulticastSession serviceIdentifier",
            "x:MulticastSession serviceIdentifier",
        ),
        "unbound namespace prefix",
    );
    assert_xml_parse_error(
        &format!("<!DOCTYPE x [<!ENTITY e \"v\">]>{good}"),
        "DOCTYPE is rejected",
    );
    assert_xml_parse_error(&format!("{good}<Extra/>"), "second root element");
    assert_xml_parse_error(&format!("{good}trailing"), "content after the root");
    assert_xml_parse_error(
        &good.replace(
            r#"schemaVersion="2""#,
            r#"schemaVersion="2" schemaVersion="2""#,
        ),
        "duplicate attribute",
    );
}

/// Hostile input: very deep nesting is handled without recursion in the tree
/// builder (no stack overflow), ending in a structured error or a clean parse.
#[test]
fn deeply_nested_document_does_not_overflow_the_stack() {
    let depth = 100_000;
    let xml = format!(
        "<MulticastGatewayConfiguration xmlns=\"{NS}\" schemaVersion=\"2\">{}{}</MulticastGatewayConfiguration>",
        "<a>".repeat(depth),
        "</a>".repeat(depth)
    );
    // Deep nesting is skipped without recursion: a clean parse or quick-xml's
    // nesting limit, as a structured error; never a stack overflow or a panic.
    let result = MulticastGatewayConfiguration::parse_str(&xml);
    assert!(
        matches!(result, Ok(_) | Err(Error::XmlParse(_))),
        "unexpected result: {result:?}"
    );
}

/// XML 1.0 §2.2: a control character is not allowed, literally or through a
/// character reference, in text or in an attribute value.
#[test]
fn control_characters_are_rejected() {
    for bad in ["&#x1;", "&#1;", "&#xB;", "\u{1}", "&#x0;"] {
        assert_xml_parse_error(&gateway_doc("s", bad), &format!("text {bad:?}"));
        assert_xml_parse_error(&gateway_doc(bad, "u"), &format!("attribute {bad:?}"));
    }
    // The allowed whitespace controls still parse.
    let parsed = MulticastGatewayConfiguration::parse_str(&gateway_doc("a&#9;b", "&#10;")).unwrap();
    assert_eq!(parsed.sessions[0].service_identifier, "a\tb");
}

/// Content before the root other than whitespace/comments is rejected, including
/// a bare entity reference or CDATA.
#[test]
fn reference_or_cdata_before_the_root_is_rejected() {
    let good = gateway_doc("s", "u");
    assert_xml_parse_error(&format!("&amp;{good}"), "reference before root");
    assert_xml_parse_error(&format!("<![CDATA[x]]>{good}"), "CDATA before root");
}

/// Character data the parser never models — inside an extension element, inside
/// a baseline element it skips, or between elements — is validated exactly like
/// modelled text: undefined entities, forbidden character references and
/// literal control characters are errors there too.
#[test]
fn invalid_character_data_in_skipped_or_unmodelled_content_is_rejected() {
    for bad in ["&nope;", "&#1;", "&#x1;", "\u{1}", "<![CDATA[\u{1}]]>"] {
        let foreign = format!(
            r#"<MulticastGatewayConfiguration xmlns="{NS}" xmlns:x="urn:f" schemaVersion="2"><x:Foo>{bad}</x:Foo></MulticastGatewayConfiguration>"#
        );
        assert_xml_parse_error(&foreign, &format!("foreign element {bad:?}"));
        let nested = format!(
            r#"<MulticastGatewayConfiguration xmlns="{NS}" schemaVersion="2"><Unknown><Deep>{bad}</Deep></Unknown></MulticastGatewayConfiguration>"#
        );
        assert_xml_parse_error(&nested, &format!("skipped baseline element {bad:?}"));
        let between = format!(
            r#"<MulticastGatewayConfiguration xmlns="{NS}" schemaVersion="2">{bad}<MulticastSession serviceIdentifier="s"/></MulticastGatewayConfiguration>"#
        );
        assert_xml_parse_error(&between, &format!("between elements {bad:?}"));
    }
}
