//! Author a TTML document from nothing — no parsed template.
//!
//! Every element struct now implements `Default`, so callers construct
//! the tree programmatically and serialize it with `to_xml()`.
//!
//! Usage: `cargo run -p ttml-subtitle --example from_scratch`

use ttml_subtitle::{
    Document, HeadElement, LayoutElement, StylingElement,
    document::{DivElement, InlineContent, PElement, RegionElement},
    validation::{ImscVersion, Profile, Validator},
};

fn main() {
    let mut doc = Document::default();

    // Root <tt> element attributes
    doc.tt.xml_lang = Some("en".to_string());
    doc.tt.ttp_content_profiles =
        Some("http://www.w3.org/ns/ttml/profile/imsc1.1/text".to_string());

    // Build a <head> with layout
    let mut head = HeadElement::default();

    let mut layout = LayoutElement::default();
    let mut region = RegionElement::default();
    region.xml_id = Some("r1".to_string());
    region.style_attributes.tts_extent = Some("80% 80%".to_string());
    region.style_attributes.tts_origin = Some("10% 10%".to_string());
    layout.regions.push(region);
    head.layout = Some(layout);

    // Build a <styling> section with a style
    let mut styling = StylingElement::default();
    let mut style = ttml_subtitle::StyleElement::default();
    style.xml_id = Some("s1".to_string());
    style.style_attributes.tts_color = Some("yellow".to_string());
    style.style_attributes.tts_font_size = Some("24px".to_string());
    styling.styles.push(style);
    head.styling = Some(styling);

    doc.tt.head = Some(head);

    // Build <body> with two timed cues
    let mut body = ttml_subtitle::BodyElement::default();
    let mut div = DivElement::default();

    let mut p1 = PElement::default();
    p1.xml_id = Some("cue1".to_string());
    p1.region = Some("r1".to_string());
    p1.style = Some("s1".to_string());
    p1.begin = Some("0s".to_string());
    p1.end = Some("3s".to_string());
    p1.content
        .push(InlineContent::Text("First cue".to_string()));

    let mut p2 = PElement::default();
    p2.xml_id = Some("cue2".to_string());
    p2.region = Some("r1".to_string());
    p2.style = Some("s1".to_string());
    p2.begin = Some("3s".to_string());
    p2.end = Some("6s".to_string());
    p2.content.push(InlineContent::Text(
        "Second cue — built from nothing".to_string(),
    ));

    div.paragraphs.push(p1);
    div.paragraphs.push(p2);
    body.divs.push(div);
    doc.tt.body = Some(body);

    let xml = doc.to_xml();
    println!("{}", xml);

    // Prove it round-trips: serialize → re-parse → re-serialize → identical
    let doc2 = Document::parse_str(&xml).expect("re-parse from-scratch output");
    let xml2 = doc2.to_xml();
    assert_eq!(
        xml, xml2,
        "from-scratch document does not round-trip identically"
    );

    // Validate
    let validator = Validator::new(Profile::Text, ImscVersion::V1_1);
    let result = validator.validate(&doc);
    println!("\nValid IMSC 1.1 Text Profile: {}", result.valid);
}
