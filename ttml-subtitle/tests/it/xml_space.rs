//! `xml:space` parse/serialize coverage (#3080): `preserve` and `default`
//! are distinct from absence, and significant whitespace survives a
//! round trip.
#![cfg(feature = "std")]

use ttml_subtitle::document::{NS_TT, XmlSpace};
use ttml_subtitle::Document;

fn wrap(inner: &str) -> String {
    format!(r#"<tt xmlns="{NS_TT}">{inner}</tt>"#)
}

fn parse(xml: &str) -> Document {
    Document::parse_str(xml).unwrap_or_else(|e| panic!("parse: {e}\n{xml}"))
}

#[test]
fn xml_space_is_parsed_on_every_content_element() {
    let xml = wrap(
        r#"<body xml:space="preserve"><div xml:space="default"><p xml:space="preserve">a  b<span xml:space="default">c  d</span></p></div></body>"#,
    );
    let doc = parse(&xml);
    let body = doc.tt.body.as_ref().expect("body");
    assert_eq!(body.xml_space, Some(XmlSpace::Preserve));
    let div = &body.divs[0];
    assert_eq!(div.xml_space, Some(XmlSpace::Default));
    let p = &div.paragraphs[0];
    assert_eq!(p.xml_space, Some(XmlSpace::Preserve));
    let span = p
        .content
        .iter()
        .find_map(|c| match c {
            ttml_subtitle::InlineContent::Span(span) => Some(span),
            _ => None,
        })
        .expect("span");
    assert_eq!(span.xml_space, Some(XmlSpace::Default));
}

#[test]
fn xml_space_absent_is_none() {
    let xml = wrap("<body><div><p>a b</p></div></body>");
    let doc = parse(&xml);
    assert_eq!(doc.tt.xml_space, None);
    let body = doc.tt.body.as_ref().expect("body");
    assert_eq!(body.xml_space, None);
    assert_eq!(body.divs[0].xml_space, None);
    assert_eq!(body.divs[0].paragraphs[0].xml_space, None);
}

#[test]
fn xml_space_bogus_is_none() {
    let xml = wrap(r#"<body><div><p xml:space="bogus">a b</p></div></body>"#);
    let doc = parse(&xml);
    assert_eq!(doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0].xml_space, None);
}

#[test]
fn xml_space_and_significant_spaces_round_trip() {
    let xml = wrap(
        r#"<body xml:space="default"><div xml:space="preserve"><p xml:space="preserve">a  b<span xml:space="default">c  d</span></p></div></body>"#,
    );
    let mut doc = parse(&xml);
    let rendered = doc.to_xml().expect("render");
    assert!(rendered.contains(r#"xml:space="preserve""#), "{rendered}");
    assert!(rendered.contains(r#"xml:space="default""#), "{rendered}");
    assert!(rendered.contains("a  b"), "{rendered}");
    assert!(rendered.contains("c  d"), "{rendered}");

    let reparsed = parse(&rendered);
    assert_eq!(
        reparsed.tt.body.as_ref().expect("body").xml_space,
        Some(XmlSpace::Default)
    );
    let p = &reparsed.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.xml_space, Some(XmlSpace::Preserve));
    let text = p.content.iter().find_map(|c| match c {
        ttml_subtitle::InlineContent::Text(t) => Some(t.as_str()),
        _ => None,
    });
    assert_eq!(text, Some("a  b"));
}
