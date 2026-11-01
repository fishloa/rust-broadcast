//! `Document::xml_declaration` is populated from the XML declaration
//! (XML 1.0 §2.8) and round-trips through `to_xml` (#1189).
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::{Document, XmlDeclaration};

const TT: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#;
const DEFAULT_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::read_to_string(p).expect("fixture")
}

#[test]
fn real_fixtures_populate_declaration() {
    // Every committed W3C IMSC fixture opens with version 1.0 / UTF-8.
    for name in ["imsc1-activearea-001.ttml", "imsc1.1-ruby-001.ttml"] {
        let doc = Document::parse_str(&fixture(name)).unwrap();
        let d = doc.xml_declaration.expect(name);
        assert_eq!(d.version, "1.0");
        assert_eq!(d.encoding.as_deref(), Some("UTF-8"));
        assert_eq!(d.standalone, None);
    }
}

#[test]
fn full_declaration_round_trips() {
    let xml = format!(r#"<?xml version="1.1" encoding="utf-8" standalone="yes"?>{TT}"#);
    let mut doc = Document::parse_str(&xml).unwrap();
    let d = doc.xml_declaration.clone().unwrap();
    assert_eq!(d.version, "1.1");
    assert_eq!(d.encoding.as_deref(), Some("utf-8"));
    assert_eq!(d.standalone, Some(true));
    let out = doc.to_xml();
    assert!(
        out.starts_with(r#"<?xml version="1.1" encoding="utf-8" standalone="yes"?>"#),
        "{out}"
    );
    assert_eq!(Document::parse_str(&out).unwrap(), doc);
}

#[test]
fn version_only_and_standalone_no() {
    let mut doc = Document::parse_str(&format!(r#"<?xml version="1.0"?>{TT}"#)).unwrap();
    assert_eq!(doc.xml_declaration.as_ref().unwrap().encoding, None);
    assert!(doc.to_xml().starts_with(r#"<?xml version="1.0"?>"#));
    let mut doc =
        Document::parse_str(&format!(r#"<?xml version="1.0" standalone="no"?>{TT}"#)).unwrap();
    assert_eq!(
        doc.xml_declaration.as_ref().unwrap().standalone,
        Some(false)
    );
    assert!(doc.to_xml().contains(r#"standalone="no""#));
}

#[test]
fn no_declaration_parses_none_and_serializes_default() {
    let mut doc = Document::parse_str(TT).unwrap();
    assert_eq!(doc.xml_declaration, None);
    assert!(doc.to_xml().starts_with(DEFAULT_DECL));
}

#[test]
fn hand_built_document_defaults_and_overrides() {
    let mut doc = Document::new();
    assert!(doc.to_xml().starts_with(DEFAULT_DECL));
    doc.xml_declaration = Some(XmlDeclaration::default());
    assert!(doc.to_xml().starts_with(DEFAULT_DECL));
}

#[test]
fn non_utf8_encoding_is_not_echoed_into_utf8_output() {
    let mut doc = Document::parse_str(&format!(
        r#"<?xml version="1.0" encoding="ISO-8859-1"?>{TT}"#
    ))
    .unwrap();
    assert_eq!(
        doc.xml_declaration.as_ref().unwrap().encoding.as_deref(),
        Some("ISO-8859-1")
    );
    assert!(doc.to_xml().contains(r#"encoding="UTF-8""#));
}

#[test]
fn bad_standalone_is_rejected() {
    assert!(
        Document::parse_str(&format!(r#"<?xml version="1.0" standalone="maybe"?>{TT}"#)).is_err()
    );
}
