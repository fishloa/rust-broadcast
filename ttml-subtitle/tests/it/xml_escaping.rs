//! XML escaping and malformed-input gate for the `quick-xml`-backed codec: all
//! five XML specials (`& < > " '`) in attribute values and text are escaped on
//! output (exact text asserted) and re-parse to the original strings;
//! malformed documents are structured errors, never panics.
#![cfg(feature = "std")]

use ttml_subtitle::{BodyElement, DivElement, Document, Error, InlineContent, PElement};

/// All five XML specials in one string.
const SPECIALS: &str = "a&b<c>d\"e'f";
/// `SPECIALS` as quick-xml escapes it.
const SPECIALS_ESCAPED: &str = "a&amp;b&lt;c&gt;d&quot;e&apos;f";

fn document_with(region: &str, text: &str) -> Document {
    let mut doc = Document::default();
    doc.tt.xml_lang = Some("en".into());
    let mut p = PElement::default();
    p.begin = Some("0s".into());
    p.end = Some("1s".into());
    p.region = Some(region.into());
    p.content.push(InlineContent::Text(text.into()));
    let mut div = DivElement::default();
    div.paragraphs.push(p);
    let mut body = BodyElement::default();
    body.divs.push(div);
    doc.tt.body = Some(body);
    doc
}

fn first_paragraph(doc: &Document) -> &PElement {
    &doc.tt.body.as_ref().unwrap().divs[0].paragraphs[0]
}

#[test]
fn specials_in_attributes_and_text_are_escaped_and_round_trip() {
    let mut doc = document_with(SPECIALS, SPECIALS);
    let xml = doc.to_xml();
    assert!(
        xml.contains(&format!("region=\"{SPECIALS_ESCAPED}\"")),
        "attribute must be escaped on the wire:\n{xml}"
    );
    assert!(
        xml.contains(&format!(">{SPECIALS_ESCAPED}</p>")),
        "text must be escaped on the wire:\n{xml}"
    );

    let reparsed = Document::parse_str(&xml).expect("re-parse");
    let p = first_paragraph(&reparsed);
    assert_eq!(p.region.as_deref(), Some(SPECIALS));
    assert_eq!(p.content, vec![InlineContent::Text(SPECIALS.into())]);
}

/// Tabs and newlines in an attribute value are written as character references
/// so attribute-value normalization on re-parse does not turn them into spaces.
#[test]
fn whitespace_characters_in_attributes_round_trip() {
    let value = "x\ty\nz\rw";
    let mut doc = document_with(value, "t");
    let xml = doc.to_xml();
    assert!(xml.contains("region=\"x&#9;y&#10;z&#13;w\""), "{xml}");
    let reparsed = Document::parse_str(&xml).expect("re-parse");
    assert_eq!(first_paragraph(&reparsed).region.as_deref(), Some(value));
}

#[test]
fn numeric_character_references_and_cdata_decode() {
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body><div><p begin="0s" end="1s" region="a&#38;b&#x3C;c">x&#65;<![CDATA[<&>]]></p></div></body></tt>"#;
    let doc = Document::parse_str(xml).expect("parse");
    let p = first_paragraph(&doc);
    assert_eq!(p.region.as_deref(), Some("a&b<c"));
    let text: String = p
        .content
        .iter()
        .filter_map(|c| match c {
            InlineContent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "xA<&>");
}

/// Hostile input: a long run of entity references decodes without blowing up.
#[test]
fn twenty_thousand_entities_decode() {
    let mut doc = document_with("r", &"&".repeat(20_000));
    let xml = doc.to_xml();
    assert!(xml.contains(&"&amp;".repeat(20_000)));
    let reparsed = Document::parse_str(&xml).expect("re-parse");
    assert_eq!(
        first_paragraph(&reparsed).content,
        vec![InlineContent::Text("&".repeat(20_000))]
    );
}

fn assert_xml_parse_error(xml: &str, what: &str) {
    match Document::parse_str(xml) {
        Err(Error::XmlParse(_)) => {}
        other => panic!("{what}: expected Error::XmlParse, got {other:?}"),
    }
}

#[test]
fn malformed_documents_are_structured_errors() {
    let good = r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body><div><p begin="0s" end="1s">hi</p></div></body></tt>"#;
    // (Truncation at every offset is asserted in tests/truncation.rs.)
    assert_xml_parse_error("", "empty input");
    assert_xml_parse_error("not xml", "plain text");
    assert_xml_parse_error(&good[..good.len() - 4], "truncated close tag");
    assert_xml_parse_error(&good.replace("hi", "a&nope;b"), "undefined entity in text");
    assert_xml_parse_error(
        &good.replace("0s", "a&nope;b"),
        "undefined entity in attribute",
    );
    assert_xml_parse_error(&good.replace("</div>", "</span>"), "mismatched end tag");
    assert_xml_parse_error(
        &good.replace("<div>", "<x:div>"),
        "unbound namespace prefix",
    );
    assert_xml_parse_error(
        &format!("<!DOCTYPE x [<!ENTITY e \"v\">]>{good}"),
        "DOCTYPE is rejected",
    );
    assert_xml_parse_error(&format!("{good}<tt/>"), "second root element");
    assert_xml_parse_error(&format!("{good}trailing"), "content after the root");
    assert_xml_parse_error(
        &good.replace(r#"begin="0s""#, r#"begin="0s" begin="1s""#),
        "duplicate attribute",
    );
}

/// Hostile input: very deep nesting never overflows the stack (flat tree, no
/// recursion on build or drop); the typed parser then rejects or accepts it.
#[test]
fn deeply_nested_document_does_not_overflow_the_stack() {
    let depth = 100_000;
    let xml = format!(
        "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div><p>{}{}</p></div></body></tt>",
        "<span>".repeat(depth),
        "</span>".repeat(depth)
    );
    // Nested spans are bounded by MAX_NESTING_DEPTH: a structured constraint
    // error (or quick-xml's own nesting limit), never a stack overflow.
    let result = Document::parse_str(&xml);
    assert!(
        matches!(
            result,
            Err(Error::ConstraintViolation { .. }) | Err(Error::XmlParse(_))
        ),
        "unexpected result: {:?}",
        result.map(|_| ())
    );
}

/// XML 1.0 §2.2: a control character is not allowed, literally or through a
/// character reference, in text or in an attribute value.
#[test]
fn control_characters_are_rejected() {
    let doc = |attr: &str, body: &str| {
        format!(
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body><div><p begin="0s" end="1s" region="{attr}">{body}</p></div></body></tt>"#
        )
    };
    for bad in ["&#x1;", "&#1;", "&#xB;", "\u{1}", "&#x0;"] {
        assert_xml_parse_error(&doc("r", bad), &format!("text {bad:?}"));
        assert_xml_parse_error(&doc(bad, "t"), &format!("attribute {bad:?}"));
    }
    let parsed = Document::parse_str(&doc("a&#9;b", "&#10;x")).unwrap();
    assert_eq!(first_paragraph(&parsed).region.as_deref(), Some("a\tb"));
}

/// Content before the root other than whitespace/comments is rejected, including
/// a bare entity reference or CDATA.
#[test]
fn reference_or_cdata_before_the_root_is_rejected() {
    let good = r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body/></tt>"#;
    assert_xml_parse_error(&format!("&amp;{good}"), "reference before root");
    assert_xml_parse_error(&format!("<![CDATA[x]]>{good}"), "CDATA before root");
}

/// Character data the parser skips or ignores (a `<br>` body, nested markup in
/// a `<chunk>`, siblings of a wrapper root's `<tt>`) is validated like modelled
/// text: undefined entities, forbidden references and control characters are
/// errors.
#[test]
fn invalid_character_data_in_skipped_content_is_rejected() {
    const NS: &str = "http://www.w3.org/ns/ttml";
    for bad in ["&nope;", "&#1;", "&#x1;", "\u{1}", "<![CDATA[\u{1}]]>"] {
        let br = format!(
            r#"<tt xmlns="{NS}" xml:lang="en"><body><div><p begin="0s" end="1s">a<br>{bad}</br></p></div></body></tt>"#
        );
        assert_xml_parse_error(&br, &format!("br body {bad:?}"));
        let chunk = format!(
            r#"<tt xmlns="{NS}" xml:lang="en"><head><resources><data><chunk><x>{bad}</x></chunk></data></resources></head></tt>"#
        );
        assert_xml_parse_error(&chunk, &format!("chunk nested {bad:?}"));
        let wrapper = format!(r#"<w><tt xmlns="{NS}"/><other>{bad}</other></w>"#);
        assert_xml_parse_error(&wrapper, &format!("wrapper sibling {bad:?}"));
    }
}
