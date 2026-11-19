//! Typed-model coverage for every child-dispatch arm in the parser (#3079):
//! a document carrying each modeled child in each legal parent must populate
//! the typed tree, and leave `unknown_children` empty at every level.
#![cfg(feature = "std")]

use ttml_subtitle::document::{MetadataChild, NS_EBUTTM, NS_ITTM, NS_TT, NS_TTM, NS_TTS};
use ttml_subtitle::{Document, InlineContent};

fn comprehensive() -> Document {
    let xml = format!(
        r#"<tt xmlns="{NS_TT}" xmlns:ttm="{NS_TTM}" xmlns:ittm="{NS_ITTM}" xmlns:ebuttm="{NS_EBUTTM}" xmlns:tts="{NS_TTS}" xml:lang="en">
  <head>
    <ttm:title>direct head title</ttm:title>
    <ttm:desc>direct head desc</ttm:desc>
    <ttm:copyright>direct head copyright</ttm:copyright>
    <ttm:agent xml:id="ha" type="person"><ttm:name type="full">direct head agent</ttm:name></ttm:agent>
    <ttm:item name="hi">direct head item</ttm:item>
    <ttm:name type="alias">direct head name</ttm:name>
    <ebuttm:documentMetadata><ebuttm:conformsToStandard>urn:head:direct</ebuttm:conformsToStandard></ebuttm:documentMetadata>
    <ebuttm:conformsToStandard>urn:head:bare</ebuttm:conformsToStandard>
    <ittm:altText>direct head alt</ittm:altText>
    <metadata>
      <ttm:title>head title</ttm:title>
      <ttm:desc>head desc</ttm:desc>
      <ttm:copyright>head copyright</ttm:copyright>
      <ttm:agent xml:id="a1" type="person">
        <ttm:name type="full">agent name</ttm:name>
      </ttm:agent>
      <ttm:item name="direct">head item</ttm:item>
      <ttm:name type="alias">direct name</ttm:name>
      <ebuttm:documentMetadata>
        <ebuttm:conformsToStandard>urn:head:standard</ebuttm:conformsToStandard>
      </ebuttm:documentMetadata>
      <ittm:altText>head alt</ittm:altText>
      <metadata><ttm:title>nested metadata</ttm:title></metadata>
    </metadata>
    <styling>
      <initial tts:color="white"/>
      <style xml:id="s1" tts:color="red"/>
    </styling>
    <layout>
      <region xml:id="r1">
        <metadata><ttm:title>region metadata</ttm:title></metadata>
        <set begin="0s" tts:color="blue"/>
        <style xml:id="rs" tts:color="green"/>
      </region>
    </layout>
    <resources>
      <metadata><ttm:title>resources metadata</ttm:title></metadata>
      <data xml:id="d1" format="text/plain">
        <metadata><ttm:title>data metadata</ttm:title></metadata>
        <chunk xml:id="c1" encoding="base64">YWJj</chunk>
        <source xml:id="ds">
          <metadata><ttm:title>data source metadata</ttm:title></metadata>
          <data xml:id="nested-data"/>
        </source>
      </data>
      <image xml:id="i1" src="a.png">
        <metadata><ttm:title>image metadata</ttm:title></metadata>
        <ittm:altText>image alt</ittm:altText>
        <set begin="0s" tts:color="red"/>
        <source xml:id="is">
          <metadata><ttm:title>image source metadata</ttm:title></metadata>
        </source>
      </image>
      <audio xml:id="a2" src="a.wav">
        <metadata><ttm:title>audio metadata</ttm:title></metadata>
        <ittm:altText>audio alt</ittm:altText>
        <set begin="0s" tts:color="red"/>
        <source xml:id="as">
          <metadata><ttm:title>audio source metadata</ttm:title></metadata>
        </source>
      </audio>
      <font xml:id="f1" family="Test">
        <metadata><ttm:title>font metadata</ttm:title></metadata>
        <set begin="0s" tts:color="red"/>
        <source xml:id="fs">
          <metadata><ttm:title>font source metadata</ttm:title></metadata>
        </source>
      </font>
    </resources>
  </head>
  <body>
    <metadata><ttm:title>body metadata</ttm:title></metadata>
    <ttm:title>body title</ttm:title>
    <ttm:desc>body desc</ttm:desc>
    <ttm:copyright>body copyright</ttm:copyright>
    <ebuttm:documentMetadata>
      <ebuttm:conformsToStandard>urn:body:standard</ebuttm:conformsToStandard>
    </ebuttm:documentMetadata>
    <ebuttm:conformsToStandard>urn:body:bare</ebuttm:conformsToStandard>
    <ittm:altText>body alt</ittm:altText>
    <set begin="0s" tts:color="red"/>
    <div>
      <metadata><ttm:title>div metadata</ttm:title></metadata>
      <ittm:altText>div alt</ittm:altText>
      <set begin="0s" tts:color="red"/>
      <audio xml:id="da" src="d.wav">
        <metadata><ttm:title>div audio metadata</ttm:title></metadata>
        <ittm:altText>div audio alt</ittm:altText>
        <set begin="0s" tts:color="red"/>
        <source xml:id="das">
          <metadata><ttm:title>div audio source</ttm:title></metadata>
        </source>
      </audio>
      <p xml:id="p1">
        <span xml:id="sp1">
          <metadata><ttm:title>span metadata</ttm:title></metadata>
          <br/>
          <image src="inline.png">
            <metadata><ttm:title>inline image metadata</ttm:title></metadata>
          </image>
          <audio src="inline.wav">
            <metadata><ttm:title>inline audio metadata</ttm:title></metadata>
          </audio>
          text
        </span>
      </p>
    </div>
  </body>
</tt>"#
    );
    Document::parse_str(&xml).unwrap_or_else(|e| panic!("parse: {e}"))
}

fn title_of(children: &[MetadataChild]) -> Option<&str> {
    children.iter().find_map(|c| match c {
        MetadataChild::TtmTitle(t) => Some(t.text.as_str()),
        _ => None,
    })
}

fn deep_title(children: &[MetadataChild]) -> Option<&str> {
    for child in children {
        if let Some(title) = title_of(core::slice::from_ref(child)) {
            return Some(title);
        }
        if let MetadataChild::Metadata(m) = child
            && let Some(title) = deep_title(&m.children)
        {
            return Some(title);
        }
    }
    None
}

fn kinds(children: &[MetadataChild]) -> Vec<&'static str> {
    children
        .iter()
        .map(|m| match m {
            MetadataChild::Metadata(_) => "metadata",
            MetadataChild::TtmTitle(_) => "title",
            MetadataChild::TtmDesc(_) => "desc",
            MetadataChild::TtmCopyright(_) => "copyright",
            MetadataChild::TtmAgent(_) => "agent",
            MetadataChild::TtmItem(_) => "item",
            MetadataChild::TtmName(_) => "name",
            MetadataChild::EbuttmDocumentMetadata(_) => "documentMetadata",
            MetadataChild::EbuttmConformsToStandard(_) => "conformsToStandard",
            MetadataChild::IttmAltText(_) => "altText",
            MetadataChild::Unknown(_) => "unknown",
            _ => "other",
        })
        .collect()
}

#[test]
fn head_children_are_typed() {
    let doc = comprehensive();
    let head = doc.tt.head.as_ref().expect("head");
    assert!(head.unknown_children.is_empty(), "head unknowns");
    let head_kinds = kinds(&head.metadata);
    for expected in [
        "metadata",
        "title",
        "desc",
        "copyright",
        "agent",
        "item",
        "name",
        "documentMetadata",
        "conformsToStandard",
        "altText",
    ] {
        assert!(
            head_kinds.contains(&expected),
            "missing head metadata {expected}: {head_kinds:?}"
        );
    }

    let metadata = head
        .metadata
        .iter()
        .find_map(|m| match m {
            MetadataChild::Metadata(m) => Some(m),
            _ => None,
        })
        .expect("metadata child");
    assert!(metadata.unknown_children.is_empty(), "metadata unknowns");
    let nested_kinds = kinds(&metadata.children);
    for expected in [
        "metadata",
        "title",
        "desc",
        "copyright",
        "agent",
        "item",
        "name",
        "documentMetadata",
        "altText",
    ] {
        assert!(
            nested_kinds.contains(&expected),
            "missing metadata child {expected}: {nested_kinds:?}"
        );
    }
    let agent = metadata
        .children
        .iter()
        .find_map(|c| match c {
            MetadataChild::TtmAgent(a) => Some(a),
            _ => None,
        })
        .expect("agent");
    assert_eq!(agent.names.len(), 1);
    assert!(agent.unknown_children.is_empty());
    let document_metadata = metadata
        .children
        .iter()
        .find_map(|c| match c {
            MetadataChild::EbuttmDocumentMetadata(e) => Some(e),
            _ => None,
        })
        .expect("documentMetadata");
    assert!(document_metadata.unknown_children.is_empty());
    assert!(document_metadata
        .children
        .iter()
        .any(|c| matches!(c, MetadataChild::EbuttmConformsToStandard(_))));
    let nested = metadata
        .children
        .iter()
        .find_map(|c| match c {
            MetadataChild::Metadata(m) => Some(m),
            _ => None,
        })
        .expect("nested metadata");
    assert!(nested.unknown_children.is_empty());
    assert_eq!(title_of(&nested.children), Some("nested metadata"));

    let styling = head.styling.as_ref().expect("styling");
    assert_eq!(styling.initials.len(), 1);
    assert_eq!(styling.styles.len(), 1);
    assert!(styling.unknown_children.is_empty());

    let layout = head.layout.as_ref().expect("layout");
    assert_eq!(layout.regions.len(), 1);
    let region = &layout.regions[0];
    assert!(region.unknown_children.is_empty());
    assert!(!region.metadata.is_empty());
    assert_eq!(region.animations.len(), 1);
    assert_eq!(region.styles.len(), 1);
    assert!(layout.unknown_children.is_empty());

    let resources = head.resources.as_ref().expect("resources");
    assert!(resources.unknown_children.is_empty());
    assert_eq!(deep_title(&resources.metadata), Some("resources metadata"));
    assert_eq!(resources.data.len(), 1);
    assert_eq!(resources.images.len(), 1);
    assert_eq!(resources.audio.len(), 1);
    assert_eq!(resources.fonts.len(), 1);

    let data = &resources.data[0];
    assert!(data.unknown_children.is_empty());
    assert_eq!(deep_title(&data.metadata), Some("data metadata"));
    assert_eq!(data.chunks.len(), 1);
    assert_eq!(data.chunks[0].text.as_deref(), Some("YWJj"));
    assert_eq!(data.sources.len(), 1);
    assert_eq!(
        deep_title(&data.sources[0].metadata),
        Some("data source metadata")
    );
    assert!(data.sources[0].data.is_some());
    assert_eq!(
        deep_title(&data.sources[0].data.as_ref().expect("source data").metadata),
        None
    );

    let image = &resources.images[0];
    assert!(image.unknown_children.is_empty());
    assert_eq!(deep_title(&image.metadata), Some("image metadata"));
    assert_eq!(image.animations.len(), 1);
    assert_eq!(image.sources.len(), 1);

    let audio = &resources.audio[0];
    assert!(audio.unknown_children.is_empty());
    assert_eq!(deep_title(&audio.metadata), Some("audio metadata"));
    assert_eq!(audio.animations.len(), 1);
    assert_eq!(audio.sources.len(), 1);

    let font = &resources.fonts[0];
    assert!(font.unknown_children.is_empty());
    assert_eq!(deep_title(&font.metadata), Some("font metadata"));
    assert_eq!(font.animations.len(), 1);
    assert_eq!(font.sources.len(), 1);
}

#[test]
fn body_and_div_children_are_typed() {
    let doc = comprehensive();
    let body = doc.tt.body.as_ref().expect("body");
    assert!(body.unknown_children.is_empty(), "body unknowns");
    let body_kinds = kinds(&body.metadata);
    for expected in [
        "metadata",
        "title",
        "desc",
        "copyright",
        "documentMetadata",
        "conformsToStandard",
        "altText",
    ] {
        assert!(
            body_kinds.contains(&expected),
            "missing body metadata {expected}: {body_kinds:?}"
        );
    }
    assert_eq!(body.animations.len(), 1);

    let div = &body.divs[0];
    assert!(div.unknown_children.is_empty(), "div unknowns");
    assert!(!div.metadata.is_empty());
    assert_eq!(div.animations.len(), 1);
    assert_eq!(div.audio.len(), 1);
    let audio = &div.audio[0];
    assert!(audio.unknown_children.is_empty());
    assert!(!audio.metadata.is_empty());
    assert_eq!(audio.animations.len(), 1);
    assert_eq!(audio.sources.len(), 1);
}

#[test]
fn span_inline_children_are_typed() {
    let doc = comprehensive();
    let p = &doc.tt.body.as_ref().expect("body").divs[0].paragraphs[0];
    assert!(p.unknown_children.is_empty());
    let span = p
        .content
        .iter()
        .find_map(|c| match c {
            InlineContent::Span(span) => Some(span),
            _ => None,
        })
        .expect("span");
    assert!(span.unknown_children.is_empty(), "span unknowns");
    assert!(!span.metadata.is_empty());
    let inline: Vec<&InlineContent> = span
        .content
        .iter()
        .filter(|c| !matches!(c, InlineContent::Text(_)))
        .collect();
    assert!(matches!(inline[0], InlineContent::Br(_)));
    let image = match inline[1] {
        InlineContent::Image(image) => image,
        other => panic!("expected image, got {other:?}"),
    };
    assert!(!image.metadata.is_empty());
    assert!(image.unknown_children.is_empty());
    let audio = match inline[2] {
        InlineContent::Audio(audio) => audio,
        other => panic!("expected audio, got {other:?}"),
    };
    assert!(!audio.metadata.is_empty());
    assert!(audio.unknown_children.is_empty());
}
