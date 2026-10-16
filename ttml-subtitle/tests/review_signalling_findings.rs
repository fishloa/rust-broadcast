//! Adversarial-review findings (review/signalling-captions): FAILING
//! regressions for confirmed defects; production code is untouched.
#![cfg(feature = "std")]

use ttml_subtitle::Document;

const TT: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#;

/// #1189 populates `xml_declaration` from the input, but `to_xml` hands the
/// stored strings to quick-xml's `BytesDecl::new`, which writes them inside
/// `"..."` unescaped. A single-quoted pseudo-attribute may legally contain
/// `"`, so an accepted input re-serializes to a document the crate's own
/// parser rejects (here: an injected `standalone="maybe"`). The version is
/// also never checked against XML 1.0 §2.8 `VersionNum` ('1.' [0-9]+).
#[test]
fn declaration_with_quote_in_version_reparses_after_to_xml() {
    let xml = format!(r#"<?xml version='1.0" standalone="maybe' ?>{TT}"#);
    let Ok(mut doc) = Document::parse_str(&xml) else {
        return; // rejecting the malformed VersionNum on parse is also a fix
    };
    let out = doc.to_xml();
    assert!(
        Document::parse_str(&out).is_ok(),
        "to_xml produced an unparseable document:\n{out}"
    );
}

/// Silent data loss: `parse_metadata_element_impl` stores an unmodeled child
/// of `<metadata>` in `MetadataElement::unknown_children`, but the
/// `MetadataChild::Metadata` arm of `serialize_metadata_child` only walks
/// `m.children` - `unknown_children` is counted for `has_children` and then
/// never written. Real SMPTE-TT / IMSC image documents keep `<smpte:image>`
/// exactly there, so `to_xml` deletes the subtitle images. (Pre-existing in
/// 0.3.0; contradicts the #1110 TT-W1 "unmodeled content is preserved" fix.)
#[test]
fn foreign_element_inside_metadata_survives_to_xml() {
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:smpte="http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt"><head><metadata><smpte:image xml:id="img0" imagetype="PNG" encoding="Base64">iVBORw0KGgo=</smpte:image></metadata></head><body/></tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let out = doc.to_xml();
    assert!(
        out.contains("iVBORw0KGgo="),
        "<smpte:image> inside <metadata> was dropped:\n{out}"
    );
}

/// XML 1.0 §2.8: the XMLDecl may only appear at the very start of the
/// document (and a PI target matching [Xx][Mm][Ll] is reserved). A
/// declaration after a comment is not well-formed but is accepted and then
/// moved to the front by `to_xml`.
#[test]
fn declaration_not_at_document_start_is_rejected() {
    let xml = format!(r#"<!-- c --><?xml version="1.0"?>{TT}"#);
    assert!(
        Document::parse_str(&xml).is_err(),
        "a misplaced XML declaration was accepted"
    );
}
