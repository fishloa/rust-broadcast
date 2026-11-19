//! Event-reader structure and skip coverage (#3081): unknown subtrees do not
//! swallow their siblings, DOCTYPE/EOF are refused inside a skipped element,
//! and the post-root checks distinguish trailing noise from a second root.
#![cfg(feature = "std")]

use std::sync::mpsc;
use std::time::Duration;

use ttml_subtitle::document::NS_TT;
use ttml_subtitle::{Document, Error, InlineContent};

fn wrap(inner: &str) -> String {
    format!(r#"<tt xmlns="{NS_TT}" xmlns:x="urn:x">{inner}</tt>"#)
}

fn parse(xml: &str) -> Document {
    Document::parse_str(xml).unwrap_or_else(|e| panic!("parse: {e}\n{xml}"))
}

fn first_p(doc: &Document) -> &ttml_subtitle::PElement {
    &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0]
}

#[test]
fn unknown_element_with_nested_children_does_not_swallow_a_sibling() {
    let doc = parse(&wrap(
        "<body><div><p><x:u><x:v>v</x:v></x:u><span>after</span></p></div></body>",
    ));
    let p = first_p(&doc);
    let unknown = p
        .content
        .iter()
        .find_map(|c| match c {
            InlineContent::Unknown(u) => Some(u),
            _ => None,
        })
        .expect("unknown element");
    assert_eq!(unknown.local_name, "u");
    let span = p
        .content
        .iter()
        .find_map(|c| match c {
            InlineContent::Span(s) => Some(s),
            _ => None,
        })
        .expect("sibling span was swallowed");
    assert_eq!(span.content, vec![InlineContent::Text("after".into())]);
}

#[test]
fn skipped_br_content_does_not_swallow_a_sibling() {
    let doc = parse(&wrap(
        "<body><div><p><br><x:y/></br><span>after</span></p></div></body>",
    ));
    let p = first_p(&doc);
    let span = p
        .content
        .iter()
        .find_map(|c| match c {
            InlineContent::Span(s) => Some(s),
            _ => None,
        })
        .expect("span after </br> was swallowed");
    assert_eq!(span.content, vec![InlineContent::Text("after".into())]);
}

#[test]
fn doctype_inside_skipped_element_is_rejected() {
    let xml = wrap("<body><div><p><br><!DOCTYPE foo></br></p></div></body>");
    assert!(
        matches!(Document::parse_str(&xml), Err(Error::XmlParse(_))),
        "DOCTYPE inside a skipped element must be refused"
    );
}

#[test]
fn eof_inside_a_skipped_element_is_an_error() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"><body><div><p><br>"#);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Document::parse_str(&xml));
    });
    let result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("parse did not terminate at EOF inside a skipped element");
    match result {
        Err(Error::XmlParse(message)) => assert!(
            message.contains("unexpected end of input inside an element"),
            "unexpected message: {message}"
        ),
        other => panic!("expected an EOF error, got {other:?}"),
    }
}

#[test]
fn trailing_comment_and_pi_after_root_are_allowed() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"><body/></tt><!-- trailing --><?pi data?>"#);
    parse(&xml);
}

#[test]
fn second_root_element_is_rejected() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"><body/></tt><tt xmlns="{NS_TT}"><body/></tt>"#);
    match Document::parse_str(&xml) {
        Err(Error::XmlParse(message)) => {
            assert!(
                message.contains("more than one root element"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected a second-root error, got {other:?}"),
    }
}

#[test]
fn character_data_before_the_root_is_rejected() {
    let xml = format!(r#"noise<tt xmlns="{NS_TT}"><body/></tt>"#);
    assert!(matches!(Document::parse_str(&xml), Err(Error::XmlParse(_))));
}

#[test]
fn skip_over_a_deep_wrapper_unknown_still_finds_the_tt() {
    let xml = format!(
        r#"<wrapper xmlns:x="urn:x"><x:a><x:b><x:c><x:d/></x:c></x:b></x:a><tt xmlns="{NS_TT}"><body><div><p>found</p></div></body></tt></wrapper>"#
    );
    let doc = parse(&xml);
    assert_eq!(
        first_p(&doc).content,
        vec![InlineContent::Text("found".into())]
    );
}

#[test]
fn doctype_after_a_semantic_error_takes_precedence() {
    let xml = r#"<tt xmlns="urn:not-tt"><!DOCTYPE foo></tt>"#;
    match Document::parse_str(xml) {
        Err(Error::XmlParse(message)) => {
            assert!(message.contains("DOCTYPE"), "unexpected message: {message}");
        }
        other => panic!("expected a DOCTYPE error, got {other:?}"),
    }
}
