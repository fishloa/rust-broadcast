//! #1110 TT-W1/TT-W2: `<p>`/`<span>` content is whitespace-significant mixed
//! content. Serializing must not insert layout whitespace into it, must keep
//! unmodeled inline elements where they were, and must not drop a `<span>`'s
//! `<metadata>`/`<set>` children.

use ttml_subtitle::Document;
use ttml_subtitle::document::InlineContent;

const NS: &str = r#"xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:v="urn:vendor""#;

fn doc(p: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<tt {NS} xml:lang="en"><body><div>{p}</div></body></tt>"#
    )
}

/// The `<p>` element's serialized text, i.e. everything between `<p ...>` and
/// `</p>`.
fn p_inner(xml: &str) -> &str {
    let start = xml.find("<p ").expect("<p");
    let open_end = start + xml[start..].find('>').unwrap() + 1;
    let close = xml.find("</p>").expect("</p>");
    &xml[open_end..close]
}

#[test]
fn hoisted_metadata_adds_no_whitespace_text_to_a_preserve_paragraph() {
    let src = doc(
        r#"<p begin="1s" end="2s" xml:space="preserve"><metadata><ttm:title>t</ttm:title></metadata>Hello<set begin="1s" tts:color="red"/><span>s</span>T</p>"#,
    );
    let mut d = Document::parse_str(&src).unwrap();
    let out = d.to_xml();
    // Nothing but the original markup between <p> and </p>: no newline/indent.
    assert_eq!(
        p_inner(&out),
        r#"<metadata><ttm:title>t</ttm:title></metadata><set begin="1s" tts:color="red"/>Hello<span>s</span>T"#,
        "{out}"
    );
    let mut again = Document::parse_str(&out).unwrap();
    assert_eq!(d, again, "text content must be identical after a round trip");
    assert_eq!(again.to_xml(), out);
}

#[test]
fn unmodeled_inline_elements_keep_their_position() {
    let src = doc(
        r#"<p begin="1s" end="2s">Hello <v:x a="1"/> mid <span>s</span> tail <chunk/> end</p>"#,
    );
    let mut d = Document::parse_str(&src).unwrap();
    // Parsed in order: text, unknown, text, span, text, unknown, text.
    let p = &d.tt.body.as_ref().unwrap().divs[0].paragraphs[0];
    let kinds: Vec<&str> = p
        .content
        .iter()
        .map(|c| match c {
            InlineContent::Text(_) => "text",
            InlineContent::Span(_) => "span",
            InlineContent::Unknown(_) => "unknown",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        ["text", "unknown", "text", "span", "text", "unknown", "text"]
    );
    let out = d.to_xml();
    assert_eq!(
        p_inner(&out),
        r#"Hello <v:x a="1"/> mid <span>s</span> tail <chunk/> end"#,
        "{out}"
    );
    // (An element in the default namespace is re-read with the `tt` prefix the
    // output also declares, so compare the serialized form, which is stable.)
    let mut again = Document::parse_str(&out).unwrap();
    assert_eq!(again.to_xml(), out);
}

/// An unprefixed foreign element lives in the *default* namespace it declares
/// itself; written without that `xmlns` it silently became a TT-namespace
/// element. Mixed foreign content is kept verbatim.
#[test]
fn unprefixed_foreign_elements_keep_their_default_namespace() {
    let src = doc(
        r#"<p begin="1s" end="2s"><f xmlns="urn:foo">a<g/>b<h xmlns="http://www.w3.org/ns/ttml"/>c</f></p>"#,
    );
    let mut d = Document::parse_str(&src).unwrap();
    let out = d.to_xml();
    assert_eq!(
        p_inner(&out),
        r#"<f xmlns="urn:foo">a<g/>b<h xmlns="http://www.w3.org/ns/ttml"/>c</f>"#,
        "{out}"
    );
    let mut again = Document::parse_str(&out).unwrap();
    assert_eq!(again, d, "re-parse must give the identical tree");
    assert_eq!(again.to_xml(), out);
}

#[test]
fn span_metadata_and_animation_children_are_serialized() {
    let src = doc(
        r#"<p begin="1s" end="2s"><span>a<metadata><ttm:title>m</ttm:title></metadata><set begin="1s" tts:color="red"/><v:y/>b</span></p>"#,
    );
    let mut d = Document::parse_str(&src).unwrap();
    let span = match &d.tt.body.as_ref().unwrap().divs[0].paragraphs[0].content[0] {
        InlineContent::Span(s) => s.clone(),
        other => panic!("expected a span, got {other:?}"),
    };
    assert_eq!(span.metadata.len(), 1);
    assert_eq!(span.animations.len(), 1);
    let out = d.to_xml();
    assert!(out.contains("<ttm:title>m</ttm:title>"), "{out}");
    assert!(out.contains(r#"<set begin="1s" tts:color="red"/>"#), "{out}");
    assert!(out.contains("<v:y/>"), "{out}");
    assert_eq!(Document::parse_str(&out).unwrap(), d);
}
