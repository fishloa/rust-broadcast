//! Namespace preservation round trips (#3082, #1110/TT-W1): a preserved
//! prefix keeps its resolved namespace URI through parse -> serialize ->
//! parse, including inner-scope re-bindings, well-known prefix collisions,
//! the reserved `xml` prefix, and extension style namespaces.
#![cfg(feature = "std")]

use ttml_subtitle::document::{MetadataChild, NS_TT, NS_TTM};
use ttml_subtitle::{Document, InlineContent, SpanElement};

fn parse(xml: &str) -> Document {
    Document::parse_str(xml).unwrap_or_else(|e| panic!("parse: {e}\n{xml}"))
}

fn round_trip(doc: &mut Document) -> Document {
    let xml = doc.to_xml().expect("render");
    parse(&xml)
}

fn span_by_id<'a>(doc: &'a Document, id: &str) -> &'a SpanElement {
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    p.content
        .iter()
        .find_map(|c| match c {
            InlineContent::Span(s) if s.xml_id.as_deref() == Some(id) => Some(s.as_ref()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("span {id} not found"))
}

fn attr_ns(span: &SpanElement, local: &str) -> Option<String> {
    span.foreign_attributes
        .iter()
        .find(|a| a.local_name == local)
        .map(|a| a.namespace.clone().unwrap_or_default())
}

#[test]
fn inner_scope_prefix_rebinding_keeps_resolved_namespaces() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:x="urn:a" xml:lang="en"><body><div><p><span xml:id="s1" xmlns:x="urn:b" x:foo="1">a</span><span xml:id="s2" x:bar="2">b</span></p></div></body></tt>"#
    );
    let mut doc = parse(&xml);
    assert_eq!(
        attr_ns(span_by_id(&doc, "s1"), "foo").as_deref(),
        Some("urn:b")
    );
    assert_eq!(
        attr_ns(span_by_id(&doc, "s2"), "bar").as_deref(),
        Some("urn:a")
    );

    let reparsed = round_trip(&mut doc);
    assert_eq!(
        attr_ns(span_by_id(&reparsed, "s1"), "foo").as_deref(),
        Some("urn:b")
    );
    assert_eq!(
        attr_ns(span_by_id(&reparsed, "s2"), "bar").as_deref(),
        Some("urn:a")
    );
}

#[test]
fn well_known_prefixes_bound_elsewhere_keep_their_uri() {
    for (prefix, uri) in [
        ("ttp", "urn:not:ttp"),
        ("tts", "urn:not:tts"),
        ("ebutts", "urn:not:ebutts"),
    ] {
        let xml = format!(
            r#"<tt xmlns="{NS_TT}" xmlns:{prefix}="{uri}" {prefix}:weird="v"><body/></tt>"#
        );
        let mut doc = parse(&xml);
        let attr = doc
            .tt
            .foreign_attributes
            .iter()
            .find(|a| a.local_name == "weird")
            .unwrap_or_else(|| panic!("{prefix}: foreign attribute missing"));
        assert_eq!(attr.namespace.as_deref(), Some(uri), "{prefix}");

        let reparsed = round_trip(&mut doc);
        let attr = reparsed
            .tt
            .foreign_attributes
            .iter()
            .find(|a| a.local_name == "weird")
            .unwrap_or_else(|| panic!("{prefix}: foreign attribute lost on round trip"));
        assert_eq!(attr.namespace.as_deref(), Some(uri), "{prefix}");
    }
}

#[test]
fn reserved_xml_prefix_is_never_rebound() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xml:lang="en"><body><div><p xml:custom="v">x</p></div></body></tt>"#
    );
    let mut doc = parse(&xml);
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    let attr = p
        .foreign_attributes
        .iter()
        .find(|a| a.local_name == "custom")
        .expect("xml:custom preserved");
    assert_eq!(
        attr.namespace.as_deref(),
        Some("http://www.w3.org/XML/1998/namespace")
    );
    let rendered = doc.to_xml().expect("render");
    assert!(rendered.contains("xml:custom="), "{rendered}");
    assert!(!rendered.contains("xmlns:xml="), "{rendered}");
    let reparsed = parse(&rendered);
    let p = &reparsed.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    let attr = p
        .foreign_attributes
        .iter()
        .find(|a| a.local_name == "custom")
        .expect("xml:custom preserved after round trip");
    assert_eq!(
        attr.namespace.as_deref(),
        Some("http://www.w3.org/XML/1998/namespace")
    );
}

#[test]
fn extension_style_namespaces_are_declared_on_demand() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:tta="http://www.w3.org/ns/ttml#audio" xmlns:itts="http://www.w3.org/ns/ttml/profile/imsc1#styling" xmlns:ebutts="urn:ebu:tt:style"><body><div><p tta:gain="1" itts:forcedDisplay="true" ebutts:linePadding="1c">x</p></div></body></tt>"#
    );
    let mut doc = parse(&xml);
    let rendered = doc.to_xml().expect("render");
    for prefix in ["tta", "itts", "ebutts"] {
        assert!(
            rendered.contains(&format!("xmlns:{prefix}=")),
            "missing xmlns:{prefix}\n{rendered}"
        );
    }
    let reparsed = parse(&rendered);
    let p = &reparsed.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.style_attributes.tta_gain.as_deref(), Some("1"));
    assert_eq!(
        p.style_attributes.itts_forced_display.as_deref(),
        Some("true")
    );
    assert_eq!(
        p.style_attributes.ebutts_line_padding.as_deref(),
        Some("1c")
    );
}

#[test]
fn unused_extension_style_namespaces_are_not_declared() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"><body><div><p>x</p></div></body></tt>"#);
    let mut doc = parse(&xml);
    let rendered = doc.to_xml().expect("render");
    for prefix in ["tta", "itts", "ebutts", "smpte", "xlink"] {
        assert!(
            !rendered.contains(&format!("xmlns:{prefix}=")),
            "unexpected xmlns:{prefix}\n{rendered}"
        );
    }
}

fn metadata_element(doc: &Document) -> &ttml_subtitle::document::MetadataElement {
    doc.tt
        .head
        .as_ref()
        .expect("head")
        .metadata
        .iter()
        .find_map(|m| match m {
            MetadataChild::Metadata(m) => Some(m),
            _ => None,
        })
        .expect("metadata")
}

#[test]
fn modeled_attributes_keep_metadata_children_typed() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttm="{NS_TTM}" xmlns:ebuttm="urn:ebu:tt:metadata"><head><metadata><ttm:title xml:lang="en">t</ttm:title><ebuttm:conformsToStandard xml:lang="en">urn:x</ebuttm:conformsToStandard><ebuttm:documentMetadata xml:lang="en"><ebuttm:conformsToStandard>urn:y</ebuttm:conformsToStandard></ebuttm:documentMetadata></metadata></head><body/></tt>"#
    );
    let doc = parse(&xml);
    let children = &metadata_element(&doc).children;
    assert!(
        children
            .iter()
            .any(|c| matches!(c, MetadataChild::TtmTitle(_)))
    );
    assert!(
        children
            .iter()
            .any(|c| matches!(c, MetadataChild::EbuttmConformsToStandard(_)))
    );
    assert!(
        children
            .iter()
            .any(|c| matches!(c, MetadataChild::EbuttmDocumentMetadata(_)))
    );
    assert!(
        children
            .iter()
            .all(|c| !matches!(c, MetadataChild::Unknown(_)))
    );
}

#[test]
fn foreign_attribute_makes_a_metadata_child_untyped() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttm="{NS_TTM}" xmlns:foo="urn:foo"><head><metadata><ttm:title foo:bar="1">t</ttm:title></metadata></head><body/></tt>"#
    );
    let doc = parse(&xml);
    let metadata = metadata_element(&doc);
    assert!(
        !metadata.unknown_children.is_empty(),
        "expected an unknown metadata child: {:?}",
        metadata.children
    );
    assert!(
        !metadata
            .children
            .iter()
            .any(|c| matches!(c, MetadataChild::TtmTitle(_)))
    );
}

#[test]
fn wrapper_root_requires_a_real_tt_child() {
    let ok = format!(
        r#"<wrapper><other/><tt xmlns="{NS_TT}"><body><div><p>ok</p></div></body></tt></wrapper>"#
    );
    let doc = parse(&ok);
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.content, vec![InlineContent::Text("ok".into())]);

    let missing = format!(r#"<wrapper><other/></wrapper>"#);
    assert!(matches!(
        Document::parse_str(&missing),
        Err(ttml_subtitle::Error::NotTtmlRoot(_))
    ));
}

#[test]
fn vendor_prefixes_survive_across_containers() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:v="urn:one" v:root="r"><body v:body="b"><div v:div="d"><p><x:u xmlns:x="urn:three" x:a="1" v:c="2"><x:deep x:d="3">t</x:deep></x:u></p></div></body></tt>"#
    );
    let mut doc = parse(&xml);
    let reparsed = round_trip(&mut doc);

    let root = reparsed
        .tt
        .foreign_attributes
        .iter()
        .find(|a| a.local_name == "root")
        .expect("root attr");
    assert_eq!(root.namespace.as_deref(), Some("urn:one"));
    let body = reparsed.tt.body.as_ref().expect("body");
    assert!(
        body.foreign_attributes
            .iter()
            .any(|a| a.local_name == "body" && a.namespace.as_deref() == Some("urn:one"))
    );
    let div = &body.divs[0];
    assert!(
        div.foreign_attributes
            .iter()
            .any(|a| a.local_name == "div" && a.namespace.as_deref() == Some("urn:one"))
    );
    let p = &div.paragraphs[0];
    let unknown = p
        .content
        .iter()
        .find_map(|c| match c {
            InlineContent::Unknown(u) => Some(u),
            _ => None,
        })
        .expect("unknown element");
    assert_eq!(unknown.namespace.as_deref(), Some("urn:three"));
    assert!(
        unknown
            .attributes
            .iter()
            .any(|a| a.local_name == "a" && a.namespace.as_deref() == Some("urn:three"))
    );
    assert!(
        unknown
            .attributes
            .iter()
            .any(|a| a.local_name == "c" && a.namespace.as_deref() == Some("urn:one"))
    );
}

#[test]
fn inner_override_output_uses_the_winning_prefix_and_a_fallback() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:x="urn:a"><body><div><p><span xml:id="s1" xmlns:x="urn:b" x:foo="1">a</span><span xml:id="s2" x:bar="2">b</span></p></div></body></tt>"#
    );
    let mut doc = parse(&xml);
    let rendered = doc.to_xml().expect("render");
    assert!(rendered.contains(r#"xmlns:x="urn:b""#), "{rendered}");
    assert!(rendered.contains(r#"x:foo="1""#), "{rendered}");
    assert!(!rendered.contains(r#"x:bar="2""#), "{rendered}");
    assert!(rendered.contains("ttmfallback"), "{rendered}");
}

#[test]
fn xlink_namespace_is_declared_for_each_attribute() {
    for attr in ["role", "arcrole", "title", "show", "href"] {
        let xml = format!(
            r#"<tt xmlns="{NS_TT}" xmlns:xlink="http://www.w3.org/1999/xlink"><body><div><image src="i.png" xlink:{attr}="v"/></div></body></tt>"#
        );
        let mut doc = parse(&xml);
        let rendered = doc.to_xml().expect("render");
        assert!(
            rendered.contains(r#"xmlns:xlink="http://www.w3.org/1999/xlink""#),
            "{attr}: xlink not declared\n{rendered}"
        );
        let reparsed = parse(&rendered);
        let image = &reparsed.tt.body.as_ref().expect("body").divs[0].images[0];
        let value = match attr {
            "role" => image.xlink_role.as_deref(),
            "arcrole" => image.xlink_arcrole.as_deref(),
            "title" => image.xlink_title.as_deref(),
            "show" => image.xlink_show.as_deref(),
            "href" => image.xlink_href.as_deref(),
            _ => unreachable!(),
        };
        assert_eq!(value, Some("v"), "{attr}");
    }
}

#[test]
fn reserved_scoped_declarations_are_never_emitted() {
    use ttml_subtitle::document::{BodyElement, NS_XML, NS_XMLNS};

    let mut doc = Document::default();
    doc.tt.scoped_namespaces = Some(vec![
        ("xml".into(), "urn:x".into()),
        ("xmlns".into(), "urn:y".into()),
        ("foo".into(), NS_XML.into()),
        ("bar".into(), NS_XMLNS.into()),
    ]);
    doc.tt.body = Some(BodyElement::default());
    let rendered = doc.to_xml().expect("render");
    for declaration in [
        r#"xmlns:xml="urn:x""#,
        r#"xmlns:xmlns="urn:y""#,
        "xmlns:foo=",
        "xmlns:bar=",
    ] {
        assert!(
            !rendered.contains(declaration),
            "unexpected {declaration}\n{rendered}"
        );
    }
}

#[test]
fn scope_less_foreign_items_are_healed_to_the_winning_uri() {
    use ttml_subtitle::document::{
        AudioElement, BodyElement, DivElement, ImageElement, MetadataChild, MetadataElement,
        PElement, SpanElement,
    };
    use ttml_subtitle::{ForeignAttribute, UnknownElement};

    fn attr(local: &str) -> ForeignAttribute {
        ForeignAttribute::new(Some("x"), Some("urn:a"), local, "1")
    }

    let mut doc = Document::default();
    doc.tt.scoped_namespaces = Some(vec![("x".into(), "urn:b".into())]);

    let mut span = SpanElement::default();
    span.foreign_attributes.push(attr("span"));
    let mut image = ImageElement::default();
    image.foreign_attributes.push(attr("image"));
    let mut audio = AudioElement::default();
    audio.foreign_attributes.push(attr("audio"));
    let mut inline_unknown = UnknownElement::new(Some("x"), Some("urn:a"), "inline");
    inline_unknown.attributes.push(attr("inline-attr"));
    inline_unknown.scoped_namespaces = Some(Vec::new());
    let mut metadata = MetadataElement::default();
    metadata.foreign_attributes.push(attr("metadata"));
    let mut metadata_unknown = UnknownElement::new(None, None, "meta-unknown");
    metadata_unknown.attributes.push(attr("meta-unknown-attr"));
    metadata_unknown.scoped_namespaces = Some(Vec::new());

    let mut p = PElement::default();
    p.foreign_attributes.push(attr("p"));
    p.content.push(InlineContent::Span(Box::new(span)));
    p.content.push(InlineContent::Image(Box::new(image)));
    p.content.push(InlineContent::Audio(Box::new(audio)));
    p.content
        .push(InlineContent::Unknown(Box::new(inline_unknown)));
    p.metadata.push(MetadataChild::Metadata(metadata));
    p.metadata.push(MetadataChild::Unknown(metadata_unknown));

    let mut div = DivElement::default();
    div.paragraphs.push(p);
    let mut body = BodyElement::default();
    body.divs.push(div);
    doc.tt.body = Some(body);

    doc.to_xml().expect("render");

    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.foreign_attributes[0].namespace.as_deref(), Some("urn:b"));
    let ns = |attrs: &[ForeignAttribute]| attrs[0].namespace.clone();
    let span = match &p.content[0] {
        InlineContent::Span(s) => ns(&s.foreign_attributes),
        other => panic!("span: {other:?}"),
    };
    assert_eq!(span.as_deref(), Some("urn:b"));
    let image = match &p.content[1] {
        InlineContent::Image(i) => ns(&i.foreign_attributes),
        other => panic!("image: {other:?}"),
    };
    assert_eq!(image.as_deref(), Some("urn:b"));
    let audio = match &p.content[2] {
        InlineContent::Audio(a) => ns(&a.foreign_attributes),
        other => panic!("audio: {other:?}"),
    };
    assert_eq!(audio.as_deref(), Some("urn:b"));
    match &p.content[3] {
        InlineContent::Unknown(u) => {
            assert_eq!(u.namespace.as_deref(), Some("urn:b"));
            assert_eq!(u.attributes[0].namespace.as_deref(), Some("urn:b"));
        }
        other => panic!("unknown: {other:?}"),
    }
    match &p.metadata[0] {
        MetadataChild::Metadata(m) => {
            assert_eq!(m.foreign_attributes[0].namespace.as_deref(), Some("urn:b"));
        }
        other => panic!("metadata: {other:?}"),
    }
    match &p.metadata[1] {
        MetadataChild::Unknown(u) => {
            assert_eq!(u.attributes[0].namespace.as_deref(), Some("urn:b"));
        }
        other => panic!("metadata unknown: {other:?}"),
    }
}
