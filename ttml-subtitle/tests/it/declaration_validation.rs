//! XML 1.0 §2.8 / §4.3.3 validation of the XML declaration (review finding 2).
#![cfg(feature = "std")]

use ttml_subtitle::{Document, Error, XmlDeclaration};

const TT: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#;

#[test]
fn malformed_declaration_parts_are_rejected() {
    const VERSION: &str = "XML declaration version must be '1.' [0-9]+";
    const ENC_NAME: &str = "XML declaration encoding is not a valid EncName";
    for (decl, expected) in [
        (r#"<?xml version="1.x"?>"#, VERSION),
        (r#"<?xml version="1."?>"#, VERSION),
        (r#"<?xml version="2.0"?>"#, VERSION),
        (r#"<?xml version="1.0" encoding="UTF 8"?>"#, ENC_NAME),
        (r#"<?xml version="1.0" encoding="8bit"?>"#, ENC_NAME),
        (r#"<?xml version='1.0" standalone="maybe' ?>"#, VERSION),
    ] {
        match Document::parse_str(&format!("{decl}{TT}")) {
            Err(Error::XmlParse(msg)) => {
                assert!(msg.starts_with(expected), "{decl}: {msg}");
            }
            other => panic!("{decl}: expected XmlParse, got {:?}", other.err()),
        }
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
        assert!(
            matches!(
                Document::parse_str(&xml),
                Err(ttml_subtitle::Error::XmlParse(..))
            ),
            "accepted: {xml}"
        );
    }
}
