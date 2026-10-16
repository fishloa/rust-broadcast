//! XML 1.0 §2.8 / §4.3.3 validation of the XML declaration (review finding 2).
#![cfg(feature = "std")]

use ttml_subtitle::{Document, XmlDeclaration};

const TT: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#;

#[test]
fn malformed_declaration_parts_are_rejected() {
    for decl in [
        r#"<?xml version="1.x"?>"#,
        r#"<?xml version="1."?>"#,
        r#"<?xml version="2.0"?>"#,
        r#"<?xml version="1.0" encoding="UTF 8"?>"#,
        r#"<?xml version="1.0" encoding="8bit"?>"#,
        r#"<?xml version='1.0" standalone="maybe' ?>"#,
    ] {
        assert!(
            Document::parse_str(&format!("{decl}{TT}")).is_err(),
            "{decl} accepted"
        );
    }
    for decl in [
        r#"<?xml version="1.0"?>"#,
        r#"<?xml version="1.10" encoding="ISO-8859-1"?>"#,
        r#"<?xml version='1.0' encoding='utf-8' standalone='no'?>"#,
    ] {
        assert!(
            Document::parse_str(&format!("{decl}{TT}")).is_ok(),
            "{decl} rejected"
        );
    }
}

#[test]
fn hand_built_bad_version_never_corrupts_the_declaration() {
    let mut doc = Document::parse_str(TT).unwrap();
    let mut d = XmlDeclaration::default();
    d.version = r#"1.0" standalone="maybe"#.into();
    doc.xml_declaration = Some(d);
    let out = doc.to_xml();
    assert!(Document::parse_str(&out).is_ok(), "{out}");
}

#[test]
fn declaration_after_whitespace_or_second_declaration_is_rejected() {
    for xml in [
        format!("\n<?xml version=\"1.0\"?>{TT}"),
        format!("<?xml version=\"1.0\"?><?xml version=\"1.0\"?>{TT}"),
        format!("<?pi x?><?xml version=\"1.0\"?>{TT}"),
    ] {
        assert!(Document::parse_str(&xml).is_err(), "accepted: {xml}");
    }
}
