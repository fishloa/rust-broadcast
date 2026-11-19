//! Parse limits (#3084): the attribute-size ceiling admits a multi-KiB
//! value, and the unknown/metadata nesting guards accept exactly 64 levels
//! and reject 65. The iterative-`Drop` guards are covered by `deep_drop`.
#![cfg(feature = "std")]

use ttml_subtitle::document::NS_TT;
use ttml_subtitle::{Document, Error, MAX_TTML_ATTRIBUTE_BYTES};

#[test]
fn attribute_size_ceiling_is_64_kib() {
    assert_eq!(MAX_TTML_ATTRIBUTE_BYTES, 65_536);

    let long = "a".repeat(4096);
    let xml = format!(
        r#"<tt xmlns="{NS_TT}"><body><div><p region="{long}">x</p></div></body></tt>"#
    );
    let doc = Document::parse_str(&xml).expect("a 4 KiB attribute value is inside the ceiling");
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.region.as_deref(), Some(long.as_str()));
}

fn nested_unknown(levels: usize) -> String {
    format!(
        r#"<tt xmlns="{NS_TT}" xmlns:x="urn:x"><body><div><p>{}{}</p></div></body></tt>"#,
        "<x:u>".repeat(levels),
        "</x:u>".repeat(levels)
    )
}

#[test]
fn unknown_nesting_accepts_64_and_rejects_65() {
    Document::parse_str(&nested_unknown(64)).expect("64 unknown levels are allowed");
    match Document::parse_str(&nested_unknown(65)) {
        Err(Error::ConstraintViolation {
            constraint,
            detail,
        }) => {
            assert_eq!(constraint, "Unknown element nesting depth limit");
            assert!(detail.contains("64"), "{detail}");
        }
        other => panic!("expected the unknown-nesting guard, got {other:?}"),
    }
}

fn nested_metadata(levels: usize) -> String {
    format!(
        r#"<tt xmlns="{NS_TT}"><head>{}{}</head><body/></tt>"#,
        "<metadata>".repeat(levels),
        "</metadata>".repeat(levels)
    )
}

#[test]
fn metadata_nesting_accepts_64_and_rejects_65() {
    Document::parse_str(&nested_metadata(64)).expect("64 metadata levels are allowed");
    match Document::parse_str(&nested_metadata(65)) {
        Err(Error::ConstraintViolation {
            constraint,
            detail,
        }) => {
            assert_eq!(constraint, "Metadata nesting depth limit");
            assert!(detail.contains("64"), "{detail}");
        }
        other => panic!("expected the metadata-nesting guard, got {other:?}"),
    }
}
