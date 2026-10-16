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
