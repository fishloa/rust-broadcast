//! Serializer shape coverage (#3083): every "has children?" decision keeps a
//! lone child kind, and the pretty-printer's exact indentation is asserted on
//! a golden document with one of each container/element kind.
#![cfg(feature = "std")]

use ttml_subtitle::document::{NS_EBUTTM, NS_ITTM, NS_SMPTE, NS_TT, NS_TTM, NS_TTS};
use ttml_subtitle::{Document, InlineContent};

fn golden_source() -> String {
    format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttm="{NS_TTM}" xmlns:ittm="{NS_ITTM}" xmlns:ebuttm="{NS_EBUTTM}" xmlns:tts="{NS_TTS}" xmlns:smpte="{NS_SMPTE}" xml:lang="en"><head><metadata><ttm:title>T</ttm:title></metadata><styling><initial tts:color="white"/><style xml:id="s" tts:color="red"/></styling><layout><region xml:id="r"><metadata><ttm:title>R</ttm:title></metadata><set begin="0s" tts:color="blue"/><style xml:id="rs" tts:color="green"/></region></layout><resources><metadata><ttm:title>Res</ttm:title></metadata><data xml:id="d" format="text/plain"><metadata><ttm:title>D</ttm:title></metadata><chunk xml:id="c" encoding="base64">YQ==</chunk><source xml:id="ds"><metadata><ttm:title>DS</ttm:title></metadata></source></data><image xml:id="i" src="i.png"><metadata><ttm:title>I</ttm:title></metadata><ittm:altText>IA</ittm:altText><set begin="0s" tts:color="red"/><source xml:id="is"><metadata><ttm:title>IS</ttm:title></metadata></source></image><audio xml:id="a" src="a.wav"><metadata><ttm:title>A</ttm:title></metadata><ittm:altText>AA</ittm:altText><set begin="0s" tts:color="red"/><source xml:id="as"><metadata><ttm:title>AS</ttm:title></metadata></source></audio><font xml:id="f" family="F"><metadata><ttm:title>F</ttm:title></metadata><set begin="0s" tts:color="red"/><source xml:id="fs"><metadata><ttm:title>FS</ttm:title></metadata></source></font></resources></head><body><metadata><ttm:title>B</ttm:title></metadata><ttm:title>BT</ttm:title><set begin="0s" tts:color="red"/><div xml:id="d1"><metadata><ttm:title>DIV</ttm:title></metadata><set begin="0s" tts:color="red"/><p xml:id="p1"><span xml:id="sp">x<br/><image src="in.png"/><audio src="in.wav">y</audio></span></p><p xml:id="empty"/></div></body></tt>"#
    )
}

fn render(xml: &str) -> String {
    let mut doc = Document::parse_str(xml).unwrap_or_else(|e| panic!("parse: {e}"));
    doc.to_xml().expect("render")
}

#[test]
fn golden_indentation_and_every_child_kind() {
    let rendered = render(&golden_source());
    assert_eq!(rendered, EXPECTED);
}

const EXPECTED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tt="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:ittm="http://www.w3.org/ns/ttml/profile/imsc1#metadata" xml:lang="en">
  <head>
    <metadata>
      <ttm:title>T</ttm:title>
    </metadata>
    <styling>
      <initial tts:color="white"/>
      <style xml:id="s" tts:color="red"/>
    </styling>
    <layout>
      <region xml:id="r">
        <metadata>
          <ttm:title>R</ttm:title>
        </metadata>
        <set begin="0s" tts:color="blue"/>
        <style xml:id="rs" tts:color="green"/>
      </region>
    </layout>
    <resources>
      <metadata>
        <ttm:title>Res</ttm:title>
      </metadata>
      <data xml:id="d" format="text/plain">
        <metadata>
          <ttm:title>D</ttm:title>
        </metadata>
        <chunk xml:id="c" encoding="base64">YQ==</chunk>
        <source xml:id="ds">
          <metadata>
            <ttm:title>DS</ttm:title>
          </metadata>
        </source>
      </data>
      <image src="i.png" xml:id="i">
        <metadata>
          <ttm:title>I</ttm:title>
        </metadata>
        <ittm:altText>IA</ittm:altText>
        <set begin="0s" tts:color="red"/>
        <source xml:id="is">
          <metadata>
            <ttm:title>IS</ttm:title>
          </metadata>
        </source>
      </image>
      <audio src="a.wav" xml:id="a">
        <metadata>
          <ttm:title>A</ttm:title>
        </metadata>
        <ittm:altText>AA</ittm:altText>
        <set begin="0s" tts:color="red"/>
        <source xml:id="as">
          <metadata>
            <ttm:title>AS</ttm:title>
          </metadata>
        </source>
      </audio>
      <font xml:id="f" family="F">
        <metadata>
          <ttm:title>F</ttm:title>
        </metadata>
        <set begin="0s" tts:color="red"/>
        <source xml:id="fs">
          <metadata>
            <ttm:title>FS</ttm:title>
          </metadata>
        </source>
      </font>
    </resources>
  </head>
  <body>
    <metadata>
      <ttm:title>B</ttm:title>
    </metadata>
    <ttm:title>BT</ttm:title>
    <set begin="0s" tts:color="red"/>
    <div xml:id="d1">
      <metadata>
        <ttm:title>DIV</ttm:title>
      </metadata>
      <set begin="0s" tts:color="red"/>
      <p xml:id="p1"><span xml:id="sp">x<br/><image src="in.png"/><audio src="in.wav">y</audio></span></p>
      <p xml:id="empty"/>
    </div>
  </body>
</tt>
"#;

#[test]
fn p_with_only_animations_keeps_them() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:tts="{NS_TTS}"><body><div><p><set begin="0s" tts:color="red"/></p></div></body></tt>"#
    );
    let rendered = render(&xml);
    assert!(rendered.contains("<set"), "{rendered}");
    let doc = Document::parse_str(&rendered).expect("reparse");
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.animations.len(), 1);
}

#[test]
fn p_with_only_metadata_keeps_it() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttm="{NS_TTM}"><body><div><p><metadata><ttm:title>t</ttm:title></metadata></p></div></body></tt>"#
    );
    let rendered = render(&xml);
    assert!(rendered.contains("<metadata"), "{rendered}");
    let doc = Document::parse_str(&rendered).expect("reparse");
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert_eq!(p.metadata.len(), 1);
}

#[test]
fn body_with_only_animations_keeps_them() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:tts="{NS_TTS}"><body><set begin="0s" tts:color="red"/></body></tt>"#
    );
    let rendered = render(&xml);
    assert!(rendered.contains("<set"), "{rendered}");
    let doc = Document::parse_str(&rendered).expect("reparse");
    assert_eq!(doc.tt.body.as_ref().expect("body").animations.len(), 1);
}

#[test]
fn div_with_only_audio_keeps_it() {
    let xml = format!(r#"<tt xmlns="{NS_TT}"><body><div><audio src="a.wav"/></div></body></tt>"#);
    let rendered = render(&xml);
    assert!(rendered.contains("<audio"), "{rendered}");
    let doc = Document::parse_str(&rendered).expect("reparse");
    assert_eq!(doc.tt.body.as_ref().expect("body").divs[0].audio.len(), 1);
}

#[test]
fn image_with_only_sources_keeps_them() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}"><head><resources><image src="i.png"><source src="s.png"/></image></resources></head><body/></tt>"#
    );
    let rendered = render(&xml);
    let doc = Document::parse_str(&rendered).expect("reparse");
    let resources = doc
        .tt
        .head
        .as_ref()
        .expect("head")
        .resources
        .as_ref()
        .expect("resources");
    assert_eq!(resources.images[0].sources.len(), 1);
}

#[test]
fn empty_p_is_self_closing_and_non_empty_p_is_not() {
    let empty = render(&format!(
        r#"<tt xmlns="{NS_TT}"><body><div><p/></div></body></tt>"#
    ));
    assert!(empty.contains("<p/>"), "{empty}");
    let text = render(&format!(
        r#"<tt xmlns="{NS_TT}"><body><div><p>x</p></div></body></tt>"#
    ));
    assert!(text.contains("<p>x</p>"), "{text}");
}

#[test]
fn chunk_with_empty_and_non_empty_text() {
    let empty = render(&format!(
        r#"<tt xmlns="{NS_TT}"><head><resources><data><chunk></chunk></data></resources></head><body/></tt>"#
    ));
    assert!(empty.contains("<chunk/>"), "{empty}");
    let text = render(&format!(
        r#"<tt xmlns="{NS_TT}"><head><resources><data><chunk>YQ==</chunk></data></resources></head><body/></tt>"#
    ));
    assert!(text.contains("<chunk>YQ==</chunk>"), "{text}");
}

#[test]
fn inline_audio_with_empty_text_is_self_closing() {
    use ttml_subtitle::document::{AudioElement, BodyElement, DivElement, PElement, SpanElement};

    let mut audio = AudioElement::default();
    audio.src = Some("a.wav".into());
    audio.text = Some(String::new());
    let mut span = SpanElement::default();
    span.content.push(InlineContent::Audio(Box::new(audio)));
    let mut p = PElement::default();
    p.content.push(InlineContent::Span(Box::new(span)));
    let mut div = DivElement::default();
    div.paragraphs.push(p);
    let mut body = BodyElement::default();
    body.divs.push(div);
    let mut doc = Document::default();
    doc.tt.body = Some(body);
    let rendered = doc.to_xml().expect("render");
    assert!(rendered.contains("<audio src=\"a.wav\"/>"), "{rendered}");
    assert!(!rendered.contains("></audio>"), "{rendered}");
}

#[test]
fn head_and_body_unknown_children_are_indented_two_spaces_per_level() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:x="urn:x"><head><x:h/></head><body><x:b/></body></tt>"#
    );
    let rendered = render(&xml);
    assert!(
        rendered.contains("  <head>\n    <x:h/>\n  </head>"),
        "{rendered}"
    );
    assert!(
        rendered.contains("  <body>\n    <x:b/>\n  </body>"),
        "{rendered}"
    );
}

#[test]
fn empty_element_prefix_is_written_unprefixed() {
    use ttml_subtitle::UnknownElement;
    use ttml_subtitle::document::BodyElement;

    let mut u = UnknownElement::new(Some(""), Some("urn:x"), "u");
    u.scoped_namespaces = Some(Vec::new());
    let mut body = BodyElement::default();
    body.unknown_children.push(u);
    let mut doc = Document::default();
    doc.tt.body = Some(body);
    let rendered = doc.to_xml().expect("render");
    assert!(rendered.contains("<u xmlns=\"urn:x\"/>"), "{rendered}");
    assert!(!rendered.contains(":u"), "{rendered}");
}

#[test]
fn empty_body_and_div_are_self_closing() {
    let body = render(&format!(r#"<tt xmlns="{NS_TT}"><body/></tt>"#));
    assert!(body.contains("<body/>"), "{body}");
    let div = render(&format!(r#"<tt xmlns="{NS_TT}"><body><div/></body></tt>"#));
    assert!(div.contains("<body>\n    <div/>\n  </body>"), "{div}");
}

#[test]
fn div_image_audio_and_unknown_child_indentation_is_exact() {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:x="urn:x"><body><div><image src="i.png"/><audio src="a.wav"/><x:u/></div></body></tt>"#
    );
    let rendered = render(&xml);
    assert!(
        rendered.contains("    <div>\n      <image src=\"i.png\"/>\n      <audio src=\"a.wav\"/>\n      <x:u/>\n    </div>"),
        "{rendered}"
    );
}
