//! TTML document structure — W3C TTML2 §3 (element syntax).
//!
//! This module defines the full element tree for a TTML2 document. Each element
//! type captures all attributes listed in its syntax box (see `ttml2-syntax.md` §3)
//! plus any namespace-qualified attributes in the TT Style Namespaces, TT Metadata
//! Namespace, and TT Parameter Namespace.
//!
//! Parsing is done via `roxmltree`; the parsed document tree is a fully typed
//! Rust structure that does NOT contain the original XML text (no raw-passthrough).

extern crate alloc;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use alloc::collections::BTreeMap;

use crate::error::{Error, Result};

// ─── Namespace constants ───────────────────────────────────────────

/// TTML namespace: `http://www.w3.org/ns/ttml`
pub const NS_TT: &str = "http://www.w3.org/ns/ttml";
/// TT Parameter namespace: `http://www.w3.org/ns/ttml#parameter`
pub const NS_TTP: &str = "http://www.w3.org/ns/ttml#parameter";
/// TT Style namespace: `http://www.w3.org/ns/ttml#styling`
pub const NS_TTS: &str = "http://www.w3.org/ns/ttml#styling";
/// TT Audio Style namespace: `http://www.w3.org/ns/ttml#audio`
pub const NS_TTA: &str = "http://www.w3.org/ns/ttml#audio";
/// TT Metadata namespace: `http://www.w3.org/ns/ttml#metadata`
pub const NS_TTM: &str = "http://www.w3.org/ns/ttml#metadata";
/// TT Profile namespace: `http://www.w3.org/ns/ttml/profile/`
pub const NS_TT_PROFILE: &str = "http://www.w3.org/ns/ttml/profile/";
/// TT Feature namespace: `http://www.w3.org/ns/ttml/feature/`
pub const NS_TT_FEATURE: &str = "http://www.w3.org/ns/ttml/feature/";
/// IMSC Styling namespace: `http://www.w3.org/ns/ttml/profile/imsc1#styling`
pub const NS_ITTS: &str = "http://www.w3.org/ns/ttml/profile/imsc1#styling";
/// IMSC Parameter namespace: `http://www.w3.org/ns/ttml/profile/imsc1#parameter`
pub const NS_ITTP: &str = "http://www.w3.org/ns/ttml/profile/imsc1#parameter";
/// IMSC Metadata namespace: `http://www.w3.org/ns/ttml/profile/imsc1#metadata`
pub const NS_ITTM: &str = "http://www.w3.org/ns/ttml/profile/imsc1#metadata";
/// EBU-TT Styling namespace: `urn:ebu:tt:style`
pub const NS_EBUTTS: &str = "urn:ebu:tt:style";
/// EBU-TT Metadata namespace: `urn:ebu:tt:metadata`
pub const NS_EBUTTM: &str = "urn:ebu:tt:metadata";
/// SMPTE-TT Extension namespace: `http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt`
pub const NS_SMPTE: &str = "http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt";
/// XML namespace: `http://www.w3.org/XML/1998/namespace`
pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";

// ─── IMSC Profile Designators ──────────────────────────────────────

/// IMSC 1.1 Text Profile designator — IMSC 1.1 §8.1.
pub const IMSC11_TEXT_PROFILE: &str = "http://www.w3.org/ns/ttml/profile/imsc1.1/text";
/// IMSC 1.1 Image Profile designator — IMSC 1.1 §9.1.
pub const IMSC11_IMAGE_PROFILE: &str = "http://www.w3.org/ns/ttml/profile/imsc1.1/image";
/// IMSC 1.0/1.0.1 Text Profile designator.
pub const IMSC1_TEXT_PROFILE: &str = "http://www.w3.org/ns/ttml/profile/imsc1/text";
/// IMSC 1.0/1.0.1 Image Profile designator.
pub const IMSC1_IMAGE_PROFILE: &str = "http://www.w3.org/ns/ttml/profile/imsc1/image";

// ─── Document root ─────────────────────────────────────────────────

/// A parsed TTML document.
///
/// This is the top-level type. Create one with [`Document::parse_str`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Document {
    /// The root `<tt>` element.
    pub tt: TtElement,
    /// Any XML declaration attributes (version, encoding).
    pub xml_declaration: Option<XmlDeclaration>,
}

/// Parsed XML declaration.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct XmlDeclaration {
    /// XML version, e.g. "1.0".
    pub version: String,
    /// XML encoding, e.g. "UTF-8".
    pub encoding: String,
}

impl Document {
    /// Create a new empty document for from-scratch construction.
    ///
    /// Use this to build TTML documents programmatically.
    /// Fields marked `#[non_exhaustive]` can be constructed using
    /// `..Default::default()` where `Default` is implemented.
    pub fn new() -> Self {
        Document {
            tt: TtElement::default(),
            xml_declaration: None,
        }
    }

    /// Parse a TTML document from an XML string.
    pub fn parse_str(xml: &str) -> Result<Self> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| Error::XmlParse(e.to_string()))?;

        let root = doc.root_element();

        // The root element might be nested if there's an XML declaration;
        // find the <tt> element.
        let tt_node = root
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "tt")
            .or_else(|| {
                if root.tag_name().name() == "tt" {
                    Some(root)
                } else {
                    None
                }
            })
            .ok_or_else(|| Error::NotTtmlRoot(root.tag_name().name().to_string()))?;

        let tt_ns = tt_node.tag_name().namespace();
        if tt_ns != Some(NS_TT) {
            return Err(Error::NotTtmlRoot(format!(
                "namespace {:?}",
                tt_ns.unwrap_or("(none)")
            )));
        }

        let tt = parse_tt_element(tt_node)?;

        Ok(Document {
            tt,
            xml_declaration: None,
        })
    }

    /// Serialize this document to an XML string.
    pub fn to_xml(&self) -> String {
        let mut buf = String::new();
        buf.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        buf.push('\n');
        serialize_tt_element(&self.tt, &mut buf, 0);
        // Ensure trailing newline
        if !buf.ends_with('\n') {
            buf.push('\n');
        }
        buf
    }

    /// Get the effective time context from the root `<tt>` element's parameter attributes.
    pub fn time_context(&self) -> crate::time::TimeContext {
        self.tt.time_context()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Element types ─────────────────────────────────────────────────

/// The root `<tt>` element — TTML2 §8.1.1.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TtElement {
    /// XML language, e.g. "en".
    pub xml_lang: Option<String>,
    /// XML id.
    pub xml_id: Option<String>,
    /// XML space: "default" or "preserve".
    pub xml_space: Option<XmlSpace>,
    /// `ttp:timeBase` — default "media".
    pub ttp_time_base: Option<String>,
    /// `ttp:frameRate` — default 30.
    pub ttp_frame_rate: Option<String>,
    /// `ttp:frameRateMultiplier` — default "1 1".
    pub ttp_frame_rate_multiplier: Option<String>,
    /// `ttp:tickRate` — default derived.
    pub ttp_tick_rate: Option<String>,
    /// `ttp:subFrameRate` — default 1.
    pub ttp_sub_frame_rate: Option<String>,
    /// `ttp:dropMode` — default "nonDrop".
    pub ttp_drop_mode: Option<String>,
    /// `ttp:markerMode` — default "discontinuous".
    pub ttp_marker_mode: Option<String>,
    /// `ttp:clockMode` — default "utc".
    pub ttp_clock_mode: Option<String>,
    /// `ttp:cellResolution` — default "32 15".
    pub ttp_cell_resolution: Option<String>,
    /// `ttp:pixelAspectRatio` — no default.
    pub ttp_pixel_aspect_ratio: Option<String>,
    /// `ttp:displayAspectRatio` — no default.
    pub ttp_display_aspect_ratio: Option<String>,
    /// `ttp:profile` attribute (the simple profile designator).
    pub ttp_profile: Option<String>,
    /// `ttp:contentProfiles` — space-separated designators or `all(...)`.
    pub ttp_content_profiles: Option<String>,
    /// `ttp:contentProfileCombination`.
    pub ttp_content_profile_combination: Option<String>,
    /// `ttp:processorProfiles`.
    pub ttp_processor_profiles: Option<String>,
    /// `ttp:processorProfileCombination`.
    pub ttp_processor_profile_combination: Option<String>,
    /// `ttp:inferProcessorProfileMethod`.
    pub ttp_infer_processor_profile_method: Option<String>,
    /// `ttp:inferProcessorProfileSource`.
    pub ttp_infer_processor_profile_source: Option<String>,
    /// `ttp:permitFeatureNarrowing`.
    pub ttp_permit_feature_narrowing: Option<String>,
    /// `ttp:permitFeatureWidening`.
    pub ttp_permit_feature_widening: Option<String>,
    /// `ttp:validation`.
    pub ttp_validation: Option<String>,
    /// `ttp:validationAction`.
    pub ttp_validation_action: Option<String>,
    /// `tts:extent` on the root element.
    pub tts_extent: Option<String>,
    /// `ittp:activeArea` — IMSC extension (IMSC 1.1 §7.8.5).
    pub ittp_active_area: Option<String>,
    /// `ittp:aspectRatio` — IMSC extension (deprecated, IMSC 1.1 §7.8.1).
    pub ittp_aspect_ratio: Option<String>,
    /// `ittp:progressivelyDecodable` — IMSC extension (IMSC 1.1 §7.8.2).
    pub ittp_progressively_decodable: Option<String>,
    /// Other attributes not covered explicitly (reserved for future extensions).
    pub other_attributes: BTreeMap<(String, String), String>,
    /// Optional `<head>` child.
    pub head: Option<HeadElement>,
    /// Optional `<body>` child.
    pub body: Option<BodyElement>,
    /// Text content (if any) — should be empty per spec.
    pub text: Option<String>,
}

impl TtElement {
    /// Derive the time context from this element's parameter attributes.
    pub fn time_context(&self) -> crate::time::TimeContext {
        use crate::time::{ClockMode, DropMode, MarkerMode, TimeBase};

        let time_base = match self.ttp_time_base.as_deref() {
            Some("smpte") => TimeBase::Smpte,
            Some("clock") => TimeBase::Clock,
            _ => TimeBase::Media,
        };

        let frame_rate: u32 = self
            .ttp_frame_rate
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        let (frame_mult_num, frame_mult_den) = if let Some(s) = &self.ttp_frame_rate_multiplier {
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() == 2 {
                (parts[0].parse().unwrap_or(1), parts[1].parse().unwrap_or(1))
            } else {
                (1, 1)
            }
        } else {
            (1, 1)
        };

        let sub_frame_rate: u32 = self
            .ttp_sub_frame_rate
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        let tick_rate: u32 = self
            .ttp_tick_rate
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(frame_rate * sub_frame_rate);

        let drop_mode = match self.ttp_drop_mode.as_deref() {
            Some("dropNTSC") => DropMode::DropNtsc,
            Some("dropPAL") => DropMode::DropPal,
            _ => DropMode::NonDrop,
        };

        let marker_mode = match self.ttp_marker_mode.as_deref() {
            Some("continuous") => MarkerMode::Continuous,
            _ => MarkerMode::Discontinuous,
        };

        let clock_mode = match self.ttp_clock_mode.as_deref() {
            Some("local") => ClockMode::Local,
            Some("gps") => ClockMode::Gps,
            _ => ClockMode::Utc,
        };

        crate::time::TimeContext {
            time_base,
            frame_rate,
            frame_rate_multiplier_numerator: frame_mult_num,
            frame_rate_multiplier_denominator: frame_mult_den,
            sub_frame_rate,
            tick_rate,
            drop_mode,
            marker_mode,
            clock_mode,
        }
    }
}

/// `<head>` element — TTML2 §8.1.2.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct HeadElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Styling section.
    pub styling: Option<StylingElement>,
    /// Layout section.
    pub layout: Option<LayoutElement>,
}

/// `<body>` element — TTML2 §8.1.3.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BodyElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`: "par" or "seq".
    pub time_container: Option<String>,
    /// `region` binding.
    pub region: Option<String>,
    /// `style` IDREFS binding.
    pub style: Option<String>,
    /// `animate` IDREFS binding.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes on the body.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
    /// Child `<div>` elements.
    pub divs: Vec<DivElement>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
}

/// `<div>` element — TTML2 §8.1.4.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct DivElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`.
    pub time_container: Option<String>,
    /// `region` IDREF.
    pub region: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
    /// SMPTE-TT `smpte:backgroundImage` attribute.
    pub smpte_background_image: Option<String>,
    /// Child `<p>` elements.
    pub paragraphs: Vec<PElement>,
    /// Child `<image>` elements.
    pub images: Vec<ImageElement>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
}

/// `<p>` element — TTML2 §8.1.5.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct PElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`.
    pub time_container: Option<String>,
    /// `region` IDREF.
    pub region: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
    /// Child content: text nodes, `<span>`, `<br>`, `<image>`, `<audio>`.
    pub content: Vec<InlineContent>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
}

/// `<span>` element — TTML2 §8.1.6.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SpanElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`.
    pub time_container: Option<String>,
    /// `region` IDREF.
    pub region: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
    /// Child content: text nodes, nested `<span>`, `<br>`.
    pub content: Vec<InlineContent>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
}

/// `<br>` element — TTML2 §8.1.7.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct BrElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
}

/// `<set>` element — TTML2 §13.1.3.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SetElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `fill` value.
    pub fill: Option<String>,
    /// `repeatCount` value.
    pub repeat_count: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
}

/// An `<image>` element — TTML2 §9.1.5 / IMSC 1.1 §9.4.4.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ImageElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`.
    pub time_container: Option<String>,
    /// `region` IDREF.
    pub region: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `src` URI.
    pub src: Option<String>,
    /// `type` MIME type.
    pub type_: Option<String>,
    /// `tts:extent` on the image.
    pub tts_extent: Option<String>,
    /// Other style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
}

/// Inline content within `<p>` and `<span>`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InlineContent {
    /// A text node (character data).
    Text(String),
    /// A `<span>` element.
    Span(Box<SpanElement>),
    /// A `<br>` element.
    Br(Box<BrElement>),
}

/// Animation children (in body, div, p, span, region).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AnimationChild {
    /// A `<set>` element.
    Set(SetElement),
}

/// Metadata children.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MetadataChild {
    /// A generic `<metadata>` element.
    Metadata(MetadataElement),
    /// `<ttm:title>` — TTML2 §14.1.8.
    TtmTitle(TtmTextElement),
    /// `<ttm:desc>` — TTML2 §14.1.5.
    TtmDesc(TtmTextElement),
    /// `<ttm:copyright>` — TTML2 §14.1.4.
    TtmCopyright(TtmTextElement),
    /// `<ttm:agent>` — TTML2 §14.1.3.
    TtmAgent(TtmAgentElement),
    /// `<ttm:item>` — TTML2 §14.1.6.
    TtmItem(TtmItemElement),
    /// `<ttm:name>` — TTML2 §14.1.7.
    TtmName(TtmNameElement),
    /// `<ebuttm:documentMetadata>` — EBU-TT-M container.
    EbuttmDocumentMetadata(EbuttmElement),
    /// `<ebuttm:conformsToStandard>` — EBU-TT-M profile signal.
    EbuttmConformsToStandard(EbuttmTextElement),
    /// `<ittm:altText>` — IMSC 1.1 §7.8.4.
    IttmAltText(IttmAltTextElement),
}

/// A `<metadata>` element — TTML2 §14.1.1.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct MetadataElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Child metadata items.
    pub children: Vec<MetadataChild>,
}

/// A text-only metadata element (`ttm:title`, `ttm:desc`, `ttm:copyright`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TtmTextElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Text content.
    pub text: String,
}

/// `<ttm:agent>` — TTML2 §14.1.3.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TtmAgentElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition`.
    pub condition: Option<String>,
    /// `type`: person, character, group, organization, other.
    pub type_: Option<String>,
    /// Child `<ttm:name>` elements.
    pub names: Vec<TtmNameElement>,
}

/// `<ttm:name>` — TTML2 §14.1.7.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TtmNameElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition`.
    pub condition: Option<String>,
    /// `type`: full, family, given, alias, other.
    pub type_: Option<String>,
    /// Text content.
    pub text: String,
}

/// `<ttm:item>` — TTML2 §14.1.6.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TtmItemElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition`.
    pub condition: Option<String>,
    /// `name` — either a named-item or QName.
    pub name: Option<String>,
    /// Text content.
    pub text: Option<String>,
    /// Nested `<ttm:item>` elements.
    pub items: Vec<TtmItemElement>,
}

/// Generic EBU-TT-M element (like `<ebuttm:documentMetadata>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EbuttmElement {
    /// Children within the EBU-TT-M element.
    pub children: Vec<MetadataChild>,
}

/// Text-only EBU-TT-M element (like `<ebuttm:conformsToStandard>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EbuttmTextElement {
    /// Text content.
    pub text: String,
}

/// `<ittm:altText>` — IMSC 1.1 §7.8.4.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct IttmAltTextElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// Text content.
    pub text: String,
}

// ─── Layout elements ───────────────────────────────────────────────

/// `<layout>` container — TTML2 §11.1.1.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct LayoutElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// Child `<region>` elements.
    pub regions: Vec<RegionElement>,
}

/// `<region>` element — TTML2 §11.1.2.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct RegionElement {
    /// XML id (required for referential binding).
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `timeContainer`.
    pub time_container: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `ttm:role`.
    pub ttm_role: Option<String>,
    /// Style attributes on the region.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
}

// ─── Styling elements ──────────────────────────────────────────────

/// `<styling>` container — TTML2 §10.1.3.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct StylingElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `<initial>` elements.
    pub initials: Vec<InitialElement>,
    /// `<style>` elements.
    pub styles: Vec<StyleElement>,
}

/// `<initial>` element — TTML2 §10.1.1.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct InitialElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition`.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
}

/// `<style>` element — TTML2 §10.1.2.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct StyleElement {
    /// XML id (required for referential binding).
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `condition`.
    pub condition: Option<String>,
    /// `style` (IDREFS, for chaining).
    pub style: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Other attributes.
    pub other_attributes: BTreeMap<(String, String), String>,
}

// ─── Style attributes ──────────────────────────────────────────────

/// All 52 TTML2 style properties (and IMSC extensions) collected in one struct.
///
/// Each field is `Option<String>` — `None` means not specified. Style property
/// names follow `ttml2-syntax.md` §3.5 (56 properties + IMSC extensions).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct StyleAttributes {
    /// `tts:backgroundColor` — TTML2 §10.2.4.
    pub tts_background_color: Option<String>,
    /// `tts:backgroundClip` — TTML2 §10.2.2.
    pub tts_background_clip: Option<String>,
    /// `tts:backgroundExtent` — TTML2 §10.2.5.
    pub tts_background_extent: Option<String>,
    /// `tts:backgroundImage` — TTML2 §10.2.6.
    pub tts_background_image: Option<String>,
    /// `tts:backgroundOrigin` — TTML2 §10.2.7.
    pub tts_background_origin: Option<String>,
    /// `tts:backgroundPosition` — TTML2 §10.2.8.
    pub tts_background_position: Option<String>,
    /// `tts:backgroundRepeat` — TTML2 §10.2.9.
    pub tts_background_repeat: Option<String>,
    /// `tts:border` — TTML2 §10.2.10.
    pub tts_border: Option<String>,
    /// `tts:bpd` — TTML2 §10.2.11.
    pub tts_bpd: Option<String>,
    /// `tts:color` — TTML2 §10.2.12.
    pub tts_color: Option<String>,
    /// `tts:direction` — TTML2 §10.2.13.
    pub tts_direction: Option<String>,
    /// `tts:disparity` — TTML2 §10.2.14.
    pub tts_disparity: Option<String>,
    /// `tts:display` — TTML2 §10.2.15.
    pub tts_display: Option<String>,
    /// `tts:displayAlign` — TTML2 §10.2.16.
    pub tts_display_align: Option<String>,
    /// `tts:extent` — TTML2 §10.2.17.
    pub tts_extent: Option<String>,
    /// `tts:fontFamily` — TTML2 §10.2.18.
    pub tts_font_family: Option<String>,
    /// `tts:fontKerning` — TTML2 §10.2.19.
    pub tts_font_kerning: Option<String>,
    /// `tts:fontSelectionStrategy` — TTML2 §10.2.20.
    pub tts_font_selection_strategy: Option<String>,
    /// `tts:fontShear` — TTML2 §10.2.21.
    pub tts_font_shear: Option<String>,
    /// `tts:fontSize` — TTML2 §10.2.22.
    pub tts_font_size: Option<String>,
    /// `tts:fontStyle` — TTML2 §10.2.23.
    pub tts_font_style: Option<String>,
    /// `tts:fontVariant` — TTML2 §10.2.24.
    pub tts_font_variant: Option<String>,
    /// `tts:fontWeight` — TTML2 §10.2.25.
    pub tts_font_weight: Option<String>,
    /// `tts:ipd` — TTML2 §10.2.26.
    pub tts_ipd: Option<String>,
    /// `tts:letterSpacing` — TTML2 §10.2.27.
    pub tts_letter_spacing: Option<String>,
    /// `tts:lineHeight` — TTML2 §10.2.28.
    pub tts_line_height: Option<String>,
    /// `tts:lineShear` — TTML2 §10.2.29.
    pub tts_line_shear: Option<String>,
    /// `tts:luminanceGain` — TTML2 §10.2.30.
    pub tts_luminance_gain: Option<String>,
    /// `tts:opacity` — TTML2 §10.2.31.
    pub tts_opacity: Option<String>,
    /// `tts:origin` — TTML2 §10.2.32.
    pub tts_origin: Option<String>,
    /// `tts:overflow` — TTML2 §10.2.33.
    pub tts_overflow: Option<String>,
    /// `tts:padding` — TTML2 §10.2.34.
    pub tts_padding: Option<String>,
    /// `tts:position` — TTML2 §10.2.35.
    pub tts_position: Option<String>,
    /// `tts:ruby` — TTML2 §10.2.36.
    pub tts_ruby: Option<String>,
    /// `tts:rubyAlign` — TTML2 §10.2.37.
    pub tts_ruby_align: Option<String>,
    /// `tts:rubyPosition` — TTML2 §10.2.38.
    pub tts_ruby_position: Option<String>,
    /// `tts:rubyReserve` — TTML2 §10.2.39.
    pub tts_ruby_reserve: Option<String>,
    /// `tts:shear` — TTML2 §10.2.40.
    pub tts_shear: Option<String>,
    /// `tts:showBackground` — TTML2 §10.2.41.
    pub tts_show_background: Option<String>,
    /// `tts:textAlign` — TTML2 §10.2.42.
    pub tts_text_align: Option<String>,
    /// `tts:textCombine` — TTML2 §10.2.43.
    pub tts_text_combine: Option<String>,
    /// `tts:textDecoration` — TTML2 §10.2.44.
    pub tts_text_decoration: Option<String>,
    /// `tts:textEmphasis` — TTML2 §10.2.45.
    pub tts_text_emphasis: Option<String>,
    /// `tts:textOrientation` — TTML2 §10.2.46.
    pub tts_text_orientation: Option<String>,
    /// `tts:textOutline` — TTML2 §10.2.47.
    pub tts_text_outline: Option<String>,
    /// `tts:textShadow` — TTML2 §10.2.48.
    pub tts_text_shadow: Option<String>,
    /// `tts:unicodeBidi` — TTML2 §10.2.49.
    pub tts_unicode_bidi: Option<String>,
    /// `tts:visibility` — TTML2 §10.2.50.
    pub tts_visibility: Option<String>,
    /// `tts:wrapOption` — TTML2 §10.2.51.
    pub tts_wrap_option: Option<String>,
    /// `tts:writingMode` — TTML2 §10.2.52.
    pub tts_writing_mode: Option<String>,
    /// `tts:zIndex` — TTML2 §10.2.53.
    pub tts_z_index: Option<String>,
    /// `tta:gain` — TTML2 §10.2.54.
    pub tta_gain: Option<String>,
    /// `tta:pan` — TTML2 §10.2.55.
    pub tta_pan: Option<String>,
    /// `tta:pitch` — TTML2 §10.2.56.
    pub tta_pitch: Option<String>,
    /// `tta:speak` — TTML2 §10.2.57.
    pub tta_speak: Option<String>,
    /// `itts:forcedDisplay` — IMSC 1.1 §7.8.3.
    pub itts_forced_display: Option<String>,
    /// `itts:fillLineGap` — IMSC 1.1 §7.8.6.
    pub itts_fill_line_gap: Option<String>,
    /// `ebutts:linePadding` — EBU-TT-D style extension.
    pub ebutts_line_padding: Option<String>,
    /// `ebutts:multiRowAlign` — EBU-TT-D style extension.
    pub ebutts_multi_row_align: Option<String>,
}

/// `xml:space` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XmlSpace {
    /// Default whitespace handling.
    Default,
    /// Preserve whitespace.
    Preserve,
}

impl XmlSpace {
    /// Label for the #204 convention.
    pub fn name(&self) -> &'static str {
        match self {
            XmlSpace::Default => "default",
            XmlSpace::Preserve => "preserve",
        }
    }
}

broadcast_common::impl_spec_display!(XmlSpace);

// ─── XML Parsing Helpers ───────────────────────────────────────────

// Removed: resolve_ns unused, has_itts unused

/// Get the prefixed name for an attribute value lookup in the document context.
fn attribute_value<'a>(node: &roxmltree::Node<'a, 'a>, ns: &str, local: &str) -> Option<&'a str> {
    // Try all attributes on the node matching ns + local_name
    for attr in node.attributes() {
        if attr.namespace() == Some(ns) && attr.name() == local {
            return Some(attr.value());
        }
    }
    None
}

/// Get all non-TT-namespace attributes for generic passthrough.
fn other_attributes(node: &roxmltree::Node<'_, '_>) -> BTreeMap<(String, String), String> {
    let known_nses = &[
        NS_TT,
        NS_TTP,
        NS_TTS,
        NS_TTA,
        NS_TTM,
        NS_TT_PROFILE,
        NS_ITTS,
        NS_ITTP,
        NS_ITTM,
        NS_EBUTTS,
        NS_EBUTTM,
        NS_SMPTE,
        NS_XML,
        "", // no-namespace attributes like begin/end/dur
    ];
    let mut map = BTreeMap::new();
    for attr in node.attributes() {
        let ns = attr.namespace().unwrap_or("");
        let local = attr.name();
        if !known_nses.contains(&ns) {
            map.insert(
                (ns.to_string(), local.to_string()),
                attr.value().to_string(),
            );
        }
    }
    map
}

// ─── XML Parsing Functions ─────────────────────────────────────────

/// Parse the root `<tt>` element from a roxmltree node.
fn parse_tt_element(node: roxmltree::Node<'_, '_>) -> Result<TtElement> {
    let mut head = None;
    let mut body = None;

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("head", Some(NS_TT)) => {
                head = Some(parse_head_element(child)?);
            }
            ("body", Some(NS_TT)) => {
                body = Some(parse_body_element(child)?);
            }
            _ => {
                // Skip unknown elements in the TT namespace (foreign elements allowed per §7.2)
            }
        }
    }

    // Parse the style attributes on tt
    let _ = parse_style_attributes(node);

    Ok(TtElement {
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        ttp_time_base: attribute_value(&node, NS_TTP, "timeBase").map(|s| s.to_string()),
        ttp_frame_rate: attribute_value(&node, NS_TTP, "frameRate").map(|s| s.to_string()),
        ttp_frame_rate_multiplier: attribute_value(&node, NS_TTP, "frameRateMultiplier")
            .map(|s| s.to_string()),
        ttp_tick_rate: attribute_value(&node, NS_TTP, "tickRate").map(|s| s.to_string()),
        ttp_sub_frame_rate: attribute_value(&node, NS_TTP, "subFrameRate").map(|s| s.to_string()),
        ttp_drop_mode: attribute_value(&node, NS_TTP, "dropMode").map(|s| s.to_string()),
        ttp_marker_mode: attribute_value(&node, NS_TTP, "markerMode").map(|s| s.to_string()),
        ttp_clock_mode: attribute_value(&node, NS_TTP, "clockMode").map(|s| s.to_string()),
        ttp_cell_resolution: attribute_value(&node, NS_TTP, "cellResolution")
            .map(|s| s.to_string()),
        ttp_pixel_aspect_ratio: attribute_value(&node, NS_TTP, "pixelAspectRatio")
            .map(|s| s.to_string()),
        ttp_display_aspect_ratio: attribute_value(&node, NS_TTP, "displayAspectRatio")
            .map(|s| s.to_string()),
        ttp_profile: attribute_value(&node, NS_TTP, "profile").map(|s| s.to_string()),
        ttp_content_profiles: attribute_value(&node, NS_TTP, "contentProfiles")
            .map(|s| s.to_string()),
        ttp_content_profile_combination: attribute_value(
            &node,
            NS_TTP,
            "contentProfileCombination",
        )
        .map(|s| s.to_string()),
        ttp_processor_profiles: attribute_value(&node, NS_TTP, "processorProfiles")
            .map(|s| s.to_string()),
        ttp_processor_profile_combination: attribute_value(
            &node,
            NS_TTP,
            "processorProfileCombination",
        )
        .map(|s| s.to_string()),
        ttp_infer_processor_profile_method: attribute_value(
            &node,
            NS_TTP,
            "inferProcessorProfileMethod",
        )
        .map(|s| s.to_string()),
        ttp_infer_processor_profile_source: attribute_value(
            &node,
            NS_TTP,
            "inferProcessorProfileSource",
        )
        .map(|s| s.to_string()),
        ttp_permit_feature_narrowing: attribute_value(&node, NS_TTP, "permitFeatureNarrowing")
            .map(|s| s.to_string()),
        ttp_permit_feature_widening: attribute_value(&node, NS_TTP, "permitFeatureWidening")
            .map(|s| s.to_string()),
        ttp_validation: attribute_value(&node, NS_TTP, "validation").map(|s| s.to_string()),
        ttp_validation_action: attribute_value(&node, NS_TTP, "validationAction")
            .map(|s| s.to_string()),
        tts_extent: attribute_value(&node, NS_TTS, "extent").map(|s| s.to_string()),
        ittp_active_area: attribute_value(&node, NS_ITTP, "activeArea").map(|s| s.to_string()),
        ittp_aspect_ratio: attribute_value(&node, NS_ITTP, "aspectRatio").map(|s| s.to_string()),
        ittp_progressively_decodable: attribute_value(&node, NS_ITTP, "progressivelyDecodable")
            .map(|s| s.to_string()),
        other_attributes: other_attributes(&node),
        head,
        body,
        text: node.text().map(|s| s.to_string()),
    })
}

fn parse_head_element(node: roxmltree::Node<'_, '_>) -> Result<HeadElement> {
    let mut metadata = Vec::new();
    let mut styling = None;
    let mut layout = None;

    // Also collect metadata from top-level
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
            }
            ("title", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmTitle(parse_ttm_text(child)?));
            }
            ("desc", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmDesc(parse_ttm_text(child)?));
            }
            ("copyright", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmCopyright(parse_ttm_text(child)?));
            }
            ("agent", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmAgent(parse_ttm_agent(child)?));
            }
            ("item", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmItem(parse_ttm_item(child)?));
            }
            ("name", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmName(parse_ttm_name(child)?));
            }
            ("documentMetadata", Some(NS_EBUTTM)) => {
                metadata.push(MetadataChild::EbuttmDocumentMetadata(parse_ebuttm_element(
                    child,
                )?));
            }
            ("conformsToStandard", Some(NS_EBUTTM)) => {
                metadata.push(MetadataChild::EbuttmConformsToStandard(parse_ebuttm_text(
                    child,
                )?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            ("styling", Some(NS_TT)) => {
                styling = Some(parse_styling_element(child)?);
            }
            ("layout", Some(NS_TT)) => {
                layout = Some(parse_layout_element(child)?);
            }
            _ => {}
        }
    }

    Ok(HeadElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        metadata,
        styling,
        layout,
    })
}

fn parse_body_element(node: roxmltree::Node<'_, '_>) -> Result<BodyElement> {
    let mut divs = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("div", Some(NS_TT)) => {
                divs.push(parse_div_element(child)?);
            }
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
            }
            ("title", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmTitle(parse_ttm_text(child)?));
            }
            ("desc", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmDesc(parse_ttm_text(child)?));
            }
            ("copyright", Some(NS_TTM)) => {
                metadata.push(MetadataChild::TtmCopyright(parse_ttm_text(child)?));
            }
            ("documentMetadata", Some(NS_EBUTTM)) => {
                metadata.push(MetadataChild::EbuttmDocumentMetadata(parse_ebuttm_element(
                    child,
                )?));
            }
            ("conformsToStandard", Some(NS_EBUTTM)) => {
                metadata.push(MetadataChild::EbuttmConformsToStandard(parse_ebuttm_text(
                    child,
                )?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(parse_set_element(child)?));
            }
            _ => {}
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(BodyElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
        divs,
        metadata,
        animations,
    })
}

fn parse_div_element(node: roxmltree::Node<'_, '_>) -> Result<DivElement> {
    let mut paragraphs = Vec::new();
    let mut images = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("p", Some(NS_TT)) => {
                paragraphs.push(parse_p_element(child)?);
            }
            ("image", Some(NS_TT)) => {
                images.push(parse_image_element(child)?);
            }
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(parse_set_element(child)?));
            }
            _ => {}
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(DivElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
        smpte_background_image: attribute_value(&node, NS_SMPTE, "backgroundImage")
            .map(|s| s.to_string()),
        paragraphs,
        images,
        metadata,
        animations,
    })
}

fn parse_p_element(node: roxmltree::Node<'_, '_>) -> Result<PElement> {
    let mut content: Vec<InlineContent> = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();

    for child in node.children() {
        if child.is_text() {
            let text = child.text().unwrap_or("");
            if !text.is_empty() {
                if let Some(last) = content.last_mut()
                    && let InlineContent::Text(t) = last
                {
                    t.push_str(text);
                    continue;
                }
                content.push(InlineContent::Text(text.to_string()));
            }
        } else if child.is_element() {
            let name = child.tag_name().name();
            let ns = child.tag_name().namespace();

            match (name, ns) {
                ("span", Some(NS_TT)) => {
                    content.push(InlineContent::Span(Box::new(parse_span_element(child)?)));
                }
                ("br", Some(NS_TT)) => {
                    content.push(InlineContent::Br(Box::new(parse_br_element(child)?)));
                }
                ("metadata", Some(NS_TT)) => {
                    metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
                }
                ("set", Some(NS_TT)) => {
                    animations.push(AnimationChild::Set(parse_set_element(child)?));
                }
                _ => {}
            }
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(PElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
        content,
        metadata,
        animations,
    })
}

fn parse_span_element(node: roxmltree::Node<'_, '_>) -> Result<SpanElement> {
    let mut content: Vec<InlineContent> = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();

    for child in node.children() {
        if child.is_text() {
            let text = child.text().unwrap_or("");
            if !text.is_empty() {
                if let Some(last) = content.last_mut()
                    && let InlineContent::Text(t) = last
                {
                    t.push_str(text);
                    continue;
                }
                content.push(InlineContent::Text(text.to_string()));
            }
        } else if child.is_element() {
            let name = child.tag_name().name();
            let ns = child.tag_name().namespace();

            match (name, ns) {
                ("span", Some(NS_TT)) => {
                    content.push(InlineContent::Span(Box::new(parse_span_element(child)?)));
                }
                ("br", Some(NS_TT)) => {
                    content.push(InlineContent::Br(Box::new(parse_br_element(child)?)));
                }
                ("metadata", Some(NS_TT)) => {
                    metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
                }
                ("set", Some(NS_TT)) => {
                    animations.push(AnimationChild::Set(parse_set_element(child)?));
                }
                _ => {}
            }
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(SpanElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
        content,
        metadata,
        animations,
    })
}

fn parse_br_element(node: roxmltree::Node<'_, '_>) -> Result<BrElement> {
    let style_attrs = parse_style_attributes(node);
    Ok(BrElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        style: node.attribute("style").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
    })
}

fn parse_set_element(node: roxmltree::Node<'_, '_>) -> Result<SetElement> {
    let style_attrs = parse_style_attributes(node);
    Ok(SetElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        fill: node.attribute("fill").map(|s| s.to_string()),
        repeat_count: node.attribute("repeatCount").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
    })
}

fn parse_image_element(node: roxmltree::Node<'_, '_>) -> Result<ImageElement> {
    let mut metadata = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(parse_metadata_element(child)?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            _ => {}
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(ImageElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        tts_extent: attribute_value(&node, NS_TTS, "extent").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
        metadata,
    })
}

fn parse_metadata_element(node: roxmltree::Node<'_, '_>) -> Result<MetadataElement> {
    let mut children = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                children.push(MetadataChild::Metadata(parse_metadata_element(child)?));
            }
            ("title", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmTitle(parse_ttm_text(child)?));
            }
            ("desc", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmDesc(parse_ttm_text(child)?));
            }
            ("copyright", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmCopyright(parse_ttm_text(child)?));
            }
            ("agent", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmAgent(parse_ttm_agent(child)?));
            }
            ("item", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmItem(parse_ttm_item(child)?));
            }
            ("name", Some(NS_TTM)) => {
                children.push(MetadataChild::TtmName(parse_ttm_name(child)?));
            }
            ("documentMetadata", Some(NS_EBUTTM)) => {
                children.push(MetadataChild::EbuttmDocumentMetadata(parse_ebuttm_element(
                    child,
                )?));
            }
            ("conformsToStandard", Some(NS_EBUTTM)) => {
                children.push(MetadataChild::EbuttmConformsToStandard(parse_ebuttm_text(
                    child,
                )?));
            }
            ("altText", Some(NS_ITTM)) => {
                children.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            _ => {}
        }
    }

    Ok(MetadataElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        children,
    })
}

fn parse_ttm_text(node: roxmltree::Node<'_, '_>) -> Result<TtmTextElement> {
    let text = node.text().unwrap_or("").to_string();
    Ok(TtmTextElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        text,
    })
}

fn parse_ttm_agent(node: roxmltree::Node<'_, '_>) -> Result<TtmAgentElement> {
    let mut names = Vec::new();
    for child in node.children() {
        if child.is_element()
            && child.tag_name().name() == "name"
            && child.tag_name().namespace() == Some(NS_TTM)
        {
            names.push(parse_ttm_name(child)?);
        }
    }
    Ok(TtmAgentElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        names,
    })
}

fn parse_ttm_name(node: roxmltree::Node<'_, '_>) -> Result<TtmNameElement> {
    let text = node.text().unwrap_or("").to_string();
    Ok(TtmNameElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        text,
    })
}

fn parse_ttm_item(node: roxmltree::Node<'_, '_>) -> Result<TtmItemElement> {
    let mut items = Vec::new();
    let mut text_parts = String::new();

    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                text_parts.push_str(t);
            }
        } else if child.is_element()
            && child.tag_name().name() == "item"
            && child.tag_name().namespace() == Some(NS_TTM)
        {
            items.push(parse_ttm_item(child)?);
        }
    }

    Ok(TtmItemElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        name: node.attribute("name").map(|s| s.to_string()),
        text: if text_parts.is_empty() {
            None
        } else {
            Some(text_parts)
        },
        items,
    })
}

fn parse_ittm_alt_text(node: roxmltree::Node<'_, '_>) -> Result<IttmAltTextElement> {
    let text = node.text().unwrap_or("").to_string();
    Ok(IttmAltTextElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        text,
    })
}

fn parse_ebuttm_element(node: roxmltree::Node<'_, '_>) -> Result<EbuttmElement> {
    let mut children = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        if name == "conformsToStandard" && ns == Some(NS_EBUTTM) {
            children.push(MetadataChild::EbuttmConformsToStandard(parse_ebuttm_text(
                child,
            )?));
        }
    }
    Ok(EbuttmElement { children })
}

fn parse_ebuttm_text(node: roxmltree::Node<'_, '_>) -> Result<EbuttmTextElement> {
    let text = node.text().unwrap_or("").to_string();
    Ok(EbuttmTextElement { text })
}

fn parse_styling_element(node: roxmltree::Node<'_, '_>) -> Result<StylingElement> {
    let mut initials = Vec::new();
    let mut styles = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("initial", Some(NS_TT)) => {
                initials.push(parse_initial_element(child)?);
            }
            ("style", Some(NS_TT)) => {
                styles.push(parse_style_element(child)?);
            }
            _ => {}
        }
    }

    Ok(StylingElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        initials,
        styles,
    })
}

fn parse_initial_element(node: roxmltree::Node<'_, '_>) -> Result<InitialElement> {
    Ok(InitialElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: parse_style_attributes(node),
        other_attributes: other_attributes(&node),
    })
}

fn parse_style_element(node: roxmltree::Node<'_, '_>) -> Result<StyleElement> {
    Ok(StyleElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        style_attributes: parse_style_attributes(node),
        other_attributes: other_attributes(&node),
    })
}

fn parse_layout_element(node: roxmltree::Node<'_, '_>) -> Result<LayoutElement> {
    let mut regions = Vec::new();
    for child in node.children() {
        if child.is_element()
            && child.tag_name().name() == "region"
            && child.tag_name().namespace() == Some(NS_TT)
        {
            regions.push(parse_region_element(child)?);
        }
    }

    Ok(LayoutElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        regions,
    })
}

fn parse_region_element(node: roxmltree::Node<'_, '_>) -> Result<RegionElement> {
    let style_attrs = parse_style_attributes(node);

    Ok(RegionElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        style_attributes: style_attrs,
        other_attributes: other_attributes(&node),
    })
}

fn parse_xml_space(value: Option<&str>) -> Option<XmlSpace> {
    match value {
        Some("preserve") => Some(XmlSpace::Preserve),
        Some("default") => Some(XmlSpace::Default),
        _ => None,
    }
}

// ─── XML Serialization Functions ──────────────────────────────────

/// Serialize the root `<tt>` element to an XML string buffer.
fn serialize_tt_element(tt: &TtElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&ind);
    buf.push_str("<tt");
    // Always output the default namespace and core namespaces
    buf.push_str(r#" xmlns="http://www.w3.org/ns/ttml""#);
    buf.push_str(r#" xmlns:tt="http://www.w3.org/ns/ttml""#);
    buf.push_str(r#" xmlns:ttp="http://www.w3.org/ns/ttml#parameter""#);
    buf.push_str(r#" xmlns:tts="http://www.w3.org/ns/ttml#styling""#);
    buf.push_str(r#" xmlns:ttm="http://www.w3.org/ns/ttml#metadata""#);

    // Only add extension namespace bindings if they're actually used
    if tt_ns_needed(tt, NS_ITTS) {
        buf.push_str(r#" xmlns:itts="http://www.w3.org/ns/ttml/profile/imsc1#styling""#);
    }
    if tt_ns_needed(tt, NS_ITTP) {
        buf.push_str(r#" xmlns:ittp="http://www.w3.org/ns/ttml/profile/imsc1#parameter""#);
    }
    if tt_ns_needed(tt, NS_ITTM) {
        buf.push_str(r#" xmlns:ittm="http://www.w3.org/ns/ttml/profile/imsc1#metadata""#);
    }
    if tt_ns_needed(tt, NS_EBUTTM) {
        buf.push_str(r#" xmlns:ebuttm="urn:ebu:tt:metadata""#);
    }
    if tt_ns_needed(tt, NS_EBUTTS) {
        buf.push_str(r#" xmlns:ebutts="urn:ebu:tt:style""#);
    }
    if tt_ns_needed(tt, NS_SMPTE) {
        buf.push_str(r#" xmlns:smpte="http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt""#);
    }
    if tt_ns_needed(tt, NS_TTA) {
        buf.push_str(r#" xmlns:tta="http://www.w3.org/ns/ttml#audio""#);
    }

    // xml:lang
    if let Some(ref lang) = tt.xml_lang {
        buf.push_str(&format!(r#" xml:lang="{}""#, xml_escape(lang)));
    }

    // ttp attributes
    serialize_opt_attr(buf, "ttp:timeBase", &tt.ttp_time_base);
    serialize_opt_attr(buf, "ttp:frameRate", &tt.ttp_frame_rate);
    serialize_opt_attr(
        buf,
        "ttp:frameRateMultiplier",
        &tt.ttp_frame_rate_multiplier,
    );
    serialize_opt_attr(buf, "ttp:tickRate", &tt.ttp_tick_rate);
    serialize_opt_attr(buf, "ttp:subFrameRate", &tt.ttp_sub_frame_rate);
    serialize_opt_attr(buf, "ttp:dropMode", &tt.ttp_drop_mode);
    serialize_opt_attr(buf, "ttp:markerMode", &tt.ttp_marker_mode);
    serialize_opt_attr(buf, "ttp:clockMode", &tt.ttp_clock_mode);
    serialize_opt_attr(buf, "ttp:cellResolution", &tt.ttp_cell_resolution);
    serialize_opt_attr(buf, "ttp:pixelAspectRatio", &tt.ttp_pixel_aspect_ratio);
    serialize_opt_attr(buf, "ttp:displayAspectRatio", &tt.ttp_display_aspect_ratio);
    serialize_opt_attr(buf, "ttp:profile", &tt.ttp_profile);
    serialize_opt_attr(buf, "ttp:contentProfiles", &tt.ttp_content_profiles);
    serialize_opt_attr(
        buf,
        "ttp:contentProfileCombination",
        &tt.ttp_content_profile_combination,
    );
    serialize_opt_attr(buf, "ttp:processorProfiles", &tt.ttp_processor_profiles);
    serialize_opt_attr(
        buf,
        "ttp:processorProfileCombination",
        &tt.ttp_processor_profile_combination,
    );
    serialize_opt_attr(
        buf,
        "ttp:permitFeatureNarrowing",
        &tt.ttp_permit_feature_narrowing,
    );
    serialize_opt_attr(
        buf,
        "ttp:permitFeatureWidening",
        &tt.ttp_permit_feature_widening,
    );
    serialize_opt_attr(buf, "ttp:validation", &tt.ttp_validation);
    serialize_opt_attr(buf, "ttp:validationAction", &tt.ttp_validation_action);

    // tts:extent on root
    serialize_opt_attr(buf, "tts:extent", &tt.tts_extent);

    // IMSC extension attributes
    serialize_opt_attr(buf, "ittp:activeArea", &tt.ittp_active_area);
    serialize_opt_attr(buf, "ittp:aspectRatio", &tt.ittp_aspect_ratio);
    serialize_opt_attr(
        buf,
        "ittp:progressivelyDecodable",
        &tt.ittp_progressively_decodable,
    );

    // xml:id
    serialize_opt_attr(buf, "xml:id", &tt.xml_id);

    buf.push_str(">\n");

    // head
    if let Some(ref head) = tt.head {
        serialize_head_element(head, buf, indent + 1);
    }

    // body
    if let Some(ref body) = tt.body {
        serialize_body_element(body, buf, indent + 1);
    }

    buf.push_str(&format!("{}</tt>\n", ind));
}

fn tt_ns_needed(tt: &TtElement, ns: &str) -> bool {
    // Check tt-level attributes
    for (attr_ns, _) in tt.other_attributes.keys() {
        if attr_ns == ns {
            return true;
        }
    }
    // Check explicit tt-level attributes
    if ns == NS_ITTP
        && (tt.ittp_active_area.is_some()
            || tt.ittp_aspect_ratio.is_some()
            || tt.ittp_progressively_decodable.is_some())
    {
        return true;
    }
    // Check children for namespace usage
    let body_needed = if let Some(ref body) = tt.body {
        body_ns_needed(body, ns)
    } else {
        false
    };
    let head_needed = if let Some(ref head) = tt.head {
        head_ns_needed(head, ns)
    } else {
        false
    };
    body_needed || head_needed
}

fn body_ns_needed(body: &BodyElement, ns: &str) -> bool {
    for (an, _) in body.other_attributes.keys() {
        if an == ns {
            return true;
        }
    }
    if style_ns_needed(&body.style_attributes, ns) {
        return true;
    }
    for div in &body.divs {
        if div_ns_needed(div, ns) {
            return true;
        }
    }
    for meta in &body.metadata {
        if meta_ns_needed(meta, ns) {
            return true;
        }
    }
    false
}

fn head_ns_needed(head: &HeadElement, ns: &str) -> bool {
    for meta in &head.metadata {
        if meta_ns_needed(meta, ns) {
            return true;
        }
    }
    false
}

fn div_ns_needed(div: &DivElement, ns: &str) -> bool {
    if div.smpte_background_image.is_some() && ns == NS_SMPTE {
        return true;
    }
    for (an, _) in div.other_attributes.keys() {
        if an == ns {
            return true;
        }
    }
    if style_ns_needed(&div.style_attributes, ns) {
        return true;
    }
    for p in &div.paragraphs {
        if style_ns_needed(&p.style_attributes, ns) {
            return true;
        }
        for (an, _) in p.other_attributes.keys() {
            if an == ns {
                return true;
            }
        }
        for item in &p.content {
            if inline_ns_needed(item, ns) {
                return true;
            }
        }
    }
    for meta in &div.metadata {
        if meta_ns_needed(meta, ns) {
            return true;
        }
    }
    for img in &div.images {
        if img.tts_extent.is_some() && ns == NS_TTS {
            return true;
        }
        if style_ns_needed(&img.style_attributes, ns) {
            return true;
        }
        for meta in &img.metadata {
            if meta_ns_needed(meta, ns) {
                return true;
            }
        }
    }
    false
}

fn inline_ns_needed(item: &InlineContent, ns: &str) -> bool {
    match item {
        InlineContent::Text(_) => false,
        InlineContent::Span(span) => {
            if style_ns_needed(&span.style_attributes, ns) {
                return true;
            }
            for (an, _) in span.other_attributes.keys() {
                if an == ns {
                    return true;
                }
            }
            for child in &span.content {
                if inline_ns_needed(child, ns) {
                    return true;
                }
            }
            false
        }
        InlineContent::Br(_) => false,
    }
}

fn meta_ns_needed(meta: &MetadataChild, ns: &str) -> bool {
    match meta {
        MetadataChild::Metadata(m) => {
            for c in &m.children {
                if meta_ns_needed(c, ns) {
                    return true;
                }
            }
            false
        }
        MetadataChild::EbuttmDocumentMetadata(eb) => {
            if ns == NS_EBUTTM {
                return true;
            }
            for c in &eb.children {
                if meta_ns_needed(c, ns) {
                    return true;
                }
            }
            false
        }
        MetadataChild::EbuttmConformsToStandard(_) => ns == NS_EBUTTM,
        MetadataChild::IttmAltText(_) => ns == NS_ITTM,
        _ => false,
    }
}

fn style_ns_needed(attrs: &StyleAttributes, ns: &str) -> bool {
    match ns {
        NS_TTS => {
            attrs.tts_background_color.is_some()
                || attrs.tts_color.is_some()
                || attrs.tts_extent.is_some()
                || attrs.tts_origin.is_some()
                || attrs.tts_font_size.is_some()
                || attrs.tts_font_family.is_some()
                || attrs.tts_font_style.is_some()
                || attrs.tts_font_weight.is_some()
                || attrs.tts_display_align.is_some()
                || attrs.tts_text_align.is_some()
                || attrs.tts_text_emphasis.is_some()
                || attrs.tts_text_shadow.is_some()
                || attrs.tts_ruby.is_some()
                || attrs.tts_ruby_align.is_some()
                || attrs.tts_ruby_position.is_some()
                || attrs.tts_ruby_reserve.is_some()
                || attrs.tts_line_height.is_some()
                || attrs.tts_opacity.is_some()
                || attrs.tts_position.is_some()
                || attrs.tts_visibility.is_some()
                || attrs.tts_display.is_some()
                || attrs.tts_writing_mode.is_some()
                || attrs.tts_show_background.is_some()
                || attrs.tts_overflow.is_some()
                || attrs.tts_z_index.is_some()
                || attrs.tts_padding.is_some()
                || attrs.tts_luminance_gain.is_some()
                || attrs.tts_direction.is_some()
                || attrs.tts_unicode_bidi.is_some()
                || attrs.tts_wrap_option.is_some()
                || attrs.tts_text_combine.is_some()
                || attrs.tts_text_decoration.is_some()
                || attrs.tts_text_orientation.is_some()
                || attrs.tts_text_outline.is_some()
                || attrs.tts_shear.is_some()
                || attrs.tts_background_clip.is_some()
                || attrs.tts_background_extent.is_some()
                || attrs.tts_background_image.is_some()
                || attrs.tts_background_origin.is_some()
                || attrs.tts_background_position.is_some()
                || attrs.tts_background_repeat.is_some()
                || attrs.tts_border.is_some()
                || attrs.tts_bpd.is_some()
                || attrs.tts_disparity.is_some()
                || attrs.tts_font_kerning.is_some()
                || attrs.tts_font_selection_strategy.is_some()
                || attrs.tts_font_shear.is_some()
                || attrs.tts_font_variant.is_some()
                || attrs.tts_ipd.is_some()
                || attrs.tts_letter_spacing.is_some()
                || attrs.tts_line_shear.is_some()
        }
        NS_ITTS => attrs.itts_forced_display.is_some() || attrs.itts_fill_line_gap.is_some(),
        NS_EBUTTS => attrs.ebutts_line_padding.is_some() || attrs.ebutts_multi_row_align.is_some(),
        NS_TTA => {
            attrs.tta_gain.is_some()
                || attrs.tta_pan.is_some()
                || attrs.tta_pitch.is_some()
                || attrs.tta_speak.is_some()
        }
        _ => false,
    }
}

fn serialize_opt_attr(buf: &mut String, name: &str, value: &Option<String>) {
    if let Some(v) = value
        && !v.is_empty()
    {
        buf.push_str(&format!(r#" {}="{}""#, name, xml_escape(v)));
    }
}

fn serialize_head_element(head: &HeadElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<head>\n", ind));

    // Metadata (ttm:title, ttm:desc, etc.)
    for meta in &head.metadata {
        serialize_metadata_child(meta, buf, indent + 1);
    }

    // Styling
    if let Some(ref styling) = head.styling {
        serialize_styling_element(styling, buf, indent + 1);
    }

    // Layout
    if let Some(ref layout) = head.layout {
        serialize_layout_element(layout, buf, indent + 1);
    }

    buf.push_str(&format!("{}</head>\n", ind));
}

fn serialize_body_element(body: &BodyElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<body", ind));

    serialize_common_timing_attrs(
        buf,
        body.begin.as_deref(),
        body.dur.as_deref(),
        body.end.as_deref(),
        body.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "region", &body.region);
    serialize_opt_attr(buf, "style", &body.style);
    serialize_opt_attr(buf, "animate", &body.animate);
    serialize_opt_attr(buf, "condition", &body.condition);
    serialize_style_attrs(&body.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &body.xml_id);
    serialize_opt_attr(buf, "xml:lang", &body.xml_lang);

    if body.divs.is_empty() && body.metadata.is_empty() && body.animations.is_empty() {
        buf.push_str("/>\n");
    } else {
        buf.push_str(">\n");

        for meta in &body.metadata {
            serialize_metadata_child(meta, buf, indent + 1);
        }
        for anim in &body.animations {
            serialize_animation_child(anim, buf, indent + 1);
        }
        for div in &body.divs {
            serialize_div_element(div, buf, indent + 1);
        }

        buf.push_str(&format!("{}</body>\n", ind));
    }
}

fn serialize_div_element(div: &DivElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<div", ind));

    serialize_common_timing_attrs(
        buf,
        div.begin.as_deref(),
        div.dur.as_deref(),
        div.end.as_deref(),
        div.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "region", &div.region);
    serialize_opt_attr(buf, "style", &div.style);
    serialize_opt_attr(buf, "animate", &div.animate);
    serialize_opt_attr(buf, "condition", &div.condition);
    serialize_style_attrs(&div.style_attributes, buf);
    serialize_opt_attr(buf, "smpte:backgroundImage", &div.smpte_background_image);
    serialize_opt_attr(buf, "xml:id", &div.xml_id);
    serialize_opt_attr(buf, "xml:lang", &div.xml_lang);

    let has_children = !div.paragraphs.is_empty()
        || !div.images.is_empty()
        || !div.metadata.is_empty()
        || !div.animations.is_empty();
    if !has_children {
        buf.push_str("/>\n");
    } else {
        buf.push_str(">\n");

        for meta in &div.metadata {
            serialize_metadata_child(meta, buf, indent + 1);
        }
        for anim in &div.animations {
            serialize_animation_child(anim, buf, indent + 1);
        }
        for p in &div.paragraphs {
            serialize_p_element(p, buf, indent + 1);
        }
        for img in &div.images {
            serialize_image_element(img, buf, indent + 1);
        }

        buf.push_str(&format!("{}</div>\n", ind));
    }
}

fn serialize_p_element(p: &PElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<p", ind));

    serialize_common_timing_attrs(
        buf,
        p.begin.as_deref(),
        p.dur.as_deref(),
        p.end.as_deref(),
        p.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "region", &p.region);
    serialize_opt_attr(buf, "style", &p.style);
    serialize_opt_attr(buf, "animate", &p.animate);
    serialize_opt_attr(buf, "condition", &p.condition);
    serialize_style_attrs(&p.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &p.xml_id);
    serialize_opt_attr(buf, "xml:lang", &p.xml_lang);

    let has_children = !p.content.is_empty() || !p.metadata.is_empty() || !p.animations.is_empty();
    if !has_children {
        buf.push_str("/>\n");
    } else {
        buf.push('>');
        for meta in &p.metadata {
            buf.push('\n');
            serialize_metadata_child(meta, buf, indent + 1);
        }
        for anim in &p.animations {
            buf.push('\n');
            serialize_animation_child(anim, buf, indent + 1);
        }
        for item in &p.content {
            serialize_inline_content(item, buf);
        }
        buf.push_str("</p>\n");
    }
}

fn serialize_inline_content(content: &InlineContent, buf: &mut String) {
    match content {
        InlineContent::Text(text) => {
            buf.push_str(&xml_escape(text));
        }
        InlineContent::Span(span) => {
            buf.push_str("<span");
            serialize_common_timing_attrs(
                buf,
                span.begin.as_deref(),
                span.dur.as_deref(),
                span.end.as_deref(),
                span.time_container.as_deref(),
            );
            serialize_opt_attr(buf, "region", &span.region);
            serialize_opt_attr(buf, "style", &span.style);
            serialize_opt_attr(buf, "animate", &span.animate);
            serialize_style_attrs(&span.style_attributes, buf);
            serialize_opt_attr(buf, "xml:id", &span.xml_id);
            serialize_opt_attr(buf, "xml:lang", &span.xml_lang);

            if span.content.is_empty() {
                buf.push_str("/>");
            } else {
                buf.push('>');
                for item in &span.content {
                    serialize_inline_content(item, buf);
                }
                buf.push_str("</span>");
            }
        }
        InlineContent::Br(_br) => {
            buf.push_str("<br/>");
        }
    }
}

fn serialize_image_element(image: &ImageElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<image", ind));

    serialize_common_timing_attrs(
        buf,
        image.begin.as_deref(),
        image.dur.as_deref(),
        image.end.as_deref(),
        image.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "region", &image.region);
    serialize_opt_attr(buf, "style", &image.style);
    serialize_opt_attr(buf, "animate", &image.animate);
    serialize_opt_attr(buf, "condition", &image.condition);
    serialize_opt_attr(buf, "src", &image.src);
    serialize_opt_attr(buf, "type", &image.type_);
    // tts:extent is explicitly on ImageElement — use that, not style_attributes
    serialize_opt_attr(buf, "tts:extent", &image.tts_extent);
    // Serialize remaining style attributes but skip tts:extent (already handled)
    serialize_style_attrs_skip_extent(&image.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &image.xml_id);
    serialize_opt_attr(buf, "xml:lang", &image.xml_lang);

    if image.metadata.is_empty() {
        buf.push_str("/>\n");
    } else {
        buf.push_str(">\n");
        for meta in &image.metadata {
            serialize_metadata_child(meta, buf, indent + 1);
        }
        buf.push_str(&format!("{}</image>\n", ind));
    }
}

fn serialize_metadata_child(child: &MetadataChild, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    match child {
        MetadataChild::Metadata(m) => {
            buf.push_str(&format!("{}<metadata>\n", ind));
            for c in &m.children {
                serialize_metadata_child(c, buf, indent + 1);
            }
            buf.push_str(&format!("{}</metadata>\n", ind));
        }
        MetadataChild::TtmTitle(t) => {
            buf.push_str(&format!(
                "{}<ttm:title>{}</ttm:title>\n",
                ind,
                xml_escape(&t.text)
            ));
        }
        MetadataChild::TtmDesc(t) => {
            buf.push_str(&format!(
                "{}<ttm:desc>{}</ttm:desc>\n",
                ind,
                xml_escape(&t.text)
            ));
        }
        MetadataChild::TtmCopyright(t) => {
            buf.push_str(&format!(
                "{}<ttm:copyright>{}</ttm:copyright>\n",
                ind,
                xml_escape(&t.text)
            ));
        }
        MetadataChild::TtmAgent(a) => {
            buf.push_str(&format!("{}<ttm:agent", ind));
            serialize_opt_attr(buf, "type", &a.type_);
            buf.push_str(">\n");
            for name in &a.names {
                buf.push_str(&format!(
                    "{}<ttm:name>{}</ttm:name>\n",
                    "  ".repeat(indent + 1),
                    xml_escape(&name.text)
                ));
            }
            buf.push_str(&format!("{}</ttm:agent>\n", ind));
        }
        MetadataChild::TtmItem(item) => {
            serialize_ttm_item(item, buf, indent);
        }
        MetadataChild::TtmName(n) => {
            buf.push_str(&format!(
                "{}<ttm:name>{}</ttm:name>\n",
                ind,
                xml_escape(&n.text)
            ));
        }
        MetadataChild::EbuttmDocumentMetadata(eb) => {
            buf.push_str(&format!("{}<ebuttm:documentMetadata>\n", ind));
            for c in &eb.children {
                serialize_metadata_child(c, buf, indent + 1);
            }
            buf.push_str(&format!("{}</ebuttm:documentMetadata>\n", ind));
        }
        MetadataChild::EbuttmConformsToStandard(cs) => {
            buf.push_str(&format!(
                "{}<ebuttm:conformsToStandard>{}</ebuttm:conformsToStandard>\n",
                ind,
                xml_escape(&cs.text)
            ));
        }
        MetadataChild::IttmAltText(alt) => {
            buf.push_str(&format!(
                "{}<ittm:altText>{}</ittm:altText>\n",
                ind,
                xml_escape(&alt.text)
            ));
        }
    }
}

fn serialize_ttm_item(item: &TtmItemElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<ttm:item", ind));
    serialize_opt_attr(buf, "name", &item.name);
    if item.items.is_empty() && item.text.is_none() {
        buf.push_str("/>\n");
    } else if item.items.is_empty() {
        buf.push('>');
        if let Some(ref t) = item.text {
            buf.push_str(&xml_escape(t));
        }
        buf.push_str("</ttm:item>\n");
    } else {
        buf.push_str(">\n");
        for i in &item.items {
            serialize_ttm_item(i, buf, indent + 1);
        }
        buf.push_str(&format!("{}</ttm:item>\n", ind));
    }
}

fn serialize_animation_child(anim: &AnimationChild, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    match anim {
        AnimationChild::Set(set) => {
            buf.push_str(&format!("{}<set", ind));
            serialize_common_timing_attrs(
                buf,
                set.begin.as_deref(),
                set.dur.as_deref(),
                set.end.as_deref(),
                None,
            );
            serialize_opt_attr(buf, "fill", &set.fill);
            serialize_opt_attr(buf, "repeatCount", &set.repeat_count);
            serialize_style_attrs(&set.style_attributes, buf);
            buf.push_str("/>\n");
        }
    }
}

fn serialize_styling_element(styling: &StylingElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<styling>\n", ind));
    for init in &styling.initials {
        buf.push_str(&format!("{}<initial", "  ".repeat(indent + 1)));
        serialize_style_attrs(&init.style_attributes, buf);
        buf.push_str("/>\n");
    }
    for style in &styling.styles {
        buf.push_str(&format!("{}<style", "  ".repeat(indent + 1)));
        serialize_opt_attr(buf, "xml:id", &style.xml_id);
        serialize_opt_attr(buf, "style", &style.style);
        serialize_style_attrs(&style.style_attributes, buf);
        buf.push_str("/>\n");
    }
    buf.push_str(&format!("{}</styling>\n", ind));
}

fn serialize_layout_element(layout: &LayoutElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<layout>\n", ind));
    for region in &layout.regions {
        serialize_region_element(region, buf, indent + 1);
    }
    buf.push_str(&format!("{}</layout>\n", ind));
}

fn serialize_region_element(region: &RegionElement, buf: &mut String, indent: usize) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{}<region", ind));

    serialize_common_timing_attrs(
        buf,
        region.begin.as_deref(),
        region.dur.as_deref(),
        region.end.as_deref(),
        region.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "style", &region.style);
    serialize_opt_attr(buf, "animate", &region.animate);
    serialize_opt_attr(buf, "condition", &region.condition);
    serialize_opt_attr(buf, "ttm:role", &region.ttm_role);
    serialize_style_attrs(&region.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &region.xml_id);
    serialize_opt_attr(buf, "xml:lang", &region.xml_lang);

    buf.push_str("/>\n");
}

fn serialize_common_timing_attrs(
    buf: &mut String,
    begin: Option<&str>,
    dur: Option<&str>,
    end: Option<&str>,
    time_container: Option<&str>,
) {
    if let Some(b) = begin {
        buf.push_str(&format!(r#" begin="{}""#, xml_escape(b)));
    }
    if let Some(d) = dur {
        buf.push_str(&format!(r#" dur="{}""#, xml_escape(d)));
    }
    if let Some(e) = end {
        buf.push_str(&format!(r#" end="{}""#, xml_escape(e)));
    }
    if let Some(tc) = time_container {
        buf.push_str(&format!(r#" timeContainer="{}""#, xml_escape(tc)));
    }
}

/// Like `serialize_style_attrs` but skips the tts:extent attribute
/// (used when tts:extent is already output via an explicit field like on ImageElement).
fn serialize_style_attrs_skip_extent(attrs: &StyleAttributes, buf: &mut String) {
    serialize_opt_attr(buf, "tts:backgroundColor", &attrs.tts_background_color);
    serialize_opt_attr(buf, "tts:backgroundClip", &attrs.tts_background_clip);
    serialize_opt_attr(buf, "tts:backgroundExtent", &attrs.tts_background_extent);
    serialize_opt_attr(buf, "tts:backgroundImage", &attrs.tts_background_image);
    serialize_opt_attr(buf, "tts:backgroundOrigin", &attrs.tts_background_origin);
    serialize_opt_attr(
        buf,
        "tts:backgroundPosition",
        &attrs.tts_background_position,
    );
    serialize_opt_attr(buf, "tts:backgroundRepeat", &attrs.tts_background_repeat);
    serialize_opt_attr(buf, "tts:border", &attrs.tts_border);
    serialize_opt_attr(buf, "tts:bpd", &attrs.tts_bpd);
    serialize_opt_attr(buf, "tts:color", &attrs.tts_color);
    serialize_opt_attr(buf, "tts:direction", &attrs.tts_direction);
    serialize_opt_attr(buf, "tts:disparity", &attrs.tts_disparity);
    serialize_opt_attr(buf, "tts:display", &attrs.tts_display);
    serialize_opt_attr(buf, "tts:displayAlign", &attrs.tts_display_align);
    serialize_opt_attr(buf, "tts:fontFamily", &attrs.tts_font_family);
    serialize_opt_attr(buf, "tts:fontKerning", &attrs.tts_font_kerning);
    serialize_opt_attr(
        buf,
        "tts:fontSelectionStrategy",
        &attrs.tts_font_selection_strategy,
    );
    serialize_opt_attr(buf, "tts:fontShear", &attrs.tts_font_shear);
    serialize_opt_attr(buf, "tts:fontSize", &attrs.tts_font_size);
    serialize_opt_attr(buf, "tts:fontStyle", &attrs.tts_font_style);
    serialize_opt_attr(buf, "tts:fontVariant", &attrs.tts_font_variant);
    serialize_opt_attr(buf, "tts:fontWeight", &attrs.tts_font_weight);
    serialize_opt_attr(buf, "tts:ipd", &attrs.tts_ipd);
    serialize_opt_attr(buf, "tts:letterSpacing", &attrs.tts_letter_spacing);
    serialize_opt_attr(buf, "tts:lineHeight", &attrs.tts_line_height);
    serialize_opt_attr(buf, "tts:lineShear", &attrs.tts_line_shear);
    serialize_opt_attr(buf, "tts:luminanceGain", &attrs.tts_luminance_gain);
    serialize_opt_attr(buf, "tts:opacity", &attrs.tts_opacity);
    serialize_opt_attr(buf, "tts:origin", &attrs.tts_origin);
    serialize_opt_attr(buf, "tts:overflow", &attrs.tts_overflow);
    serialize_opt_attr(buf, "tts:padding", &attrs.tts_padding);
    serialize_opt_attr(buf, "tts:position", &attrs.tts_position);
    serialize_opt_attr(buf, "tts:ruby", &attrs.tts_ruby);
    serialize_opt_attr(buf, "tts:rubyAlign", &attrs.tts_ruby_align);
    serialize_opt_attr(buf, "tts:rubyPosition", &attrs.tts_ruby_position);
    serialize_opt_attr(buf, "tts:rubyReserve", &attrs.tts_ruby_reserve);
    serialize_opt_attr(buf, "tts:shear", &attrs.tts_shear);
    serialize_opt_attr(buf, "tts:showBackground", &attrs.tts_show_background);
    serialize_opt_attr(buf, "tts:textAlign", &attrs.tts_text_align);
    serialize_opt_attr(buf, "tts:textCombine", &attrs.tts_text_combine);
    serialize_opt_attr(buf, "tts:textDecoration", &attrs.tts_text_decoration);
    serialize_opt_attr(buf, "tts:textEmphasis", &attrs.tts_text_emphasis);
    serialize_opt_attr(buf, "tts:textOrientation", &attrs.tts_text_orientation);
    serialize_opt_attr(buf, "tts:textOutline", &attrs.tts_text_outline);
    serialize_opt_attr(buf, "tts:textShadow", &attrs.tts_text_shadow);
    serialize_opt_attr(buf, "tts:unicodeBidi", &attrs.tts_unicode_bidi);
    serialize_opt_attr(buf, "tts:visibility", &attrs.tts_visibility);
    serialize_opt_attr(buf, "tts:wrapOption", &attrs.tts_wrap_option);
    serialize_opt_attr(buf, "tts:writingMode", &attrs.tts_writing_mode);
    serialize_opt_attr(buf, "tts:zIndex", &attrs.tts_z_index);
    serialize_opt_attr(buf, "tta:gain", &attrs.tta_gain);
    serialize_opt_attr(buf, "tta:pan", &attrs.tta_pan);
    serialize_opt_attr(buf, "tta:pitch", &attrs.tta_pitch);
    serialize_opt_attr(buf, "tta:speak", &attrs.tta_speak);
    serialize_opt_attr(buf, "itts:forcedDisplay", &attrs.itts_forced_display);
    serialize_opt_attr(buf, "itts:fillLineGap", &attrs.itts_fill_line_gap);
    serialize_opt_attr(buf, "ebutts:linePadding", &attrs.ebutts_line_padding);
    serialize_opt_attr(buf, "ebutts:multiRowAlign", &attrs.ebutts_multi_row_align);
}

#[allow(clippy::too_many_lines)]
fn serialize_style_attrs(attrs: &StyleAttributes, buf: &mut String) {
    serialize_opt_attr(buf, "tts:backgroundColor", &attrs.tts_background_color);
    serialize_opt_attr(buf, "tts:backgroundClip", &attrs.tts_background_clip);
    serialize_opt_attr(buf, "tts:backgroundExtent", &attrs.tts_background_extent);
    serialize_opt_attr(buf, "tts:backgroundImage", &attrs.tts_background_image);
    serialize_opt_attr(buf, "tts:backgroundOrigin", &attrs.tts_background_origin);
    serialize_opt_attr(
        buf,
        "tts:backgroundPosition",
        &attrs.tts_background_position,
    );
    serialize_opt_attr(buf, "tts:backgroundRepeat", &attrs.tts_background_repeat);
    serialize_opt_attr(buf, "tts:border", &attrs.tts_border);
    serialize_opt_attr(buf, "tts:bpd", &attrs.tts_bpd);
    serialize_opt_attr(buf, "tts:color", &attrs.tts_color);
    serialize_opt_attr(buf, "tts:direction", &attrs.tts_direction);
    serialize_opt_attr(buf, "tts:disparity", &attrs.tts_disparity);
    serialize_opt_attr(buf, "tts:display", &attrs.tts_display);
    serialize_opt_attr(buf, "tts:displayAlign", &attrs.tts_display_align);
    serialize_opt_attr(buf, "tts:extent", &attrs.tts_extent);
    serialize_opt_attr(buf, "tts:fontFamily", &attrs.tts_font_family);
    serialize_opt_attr(buf, "tts:fontKerning", &attrs.tts_font_kerning);
    serialize_opt_attr(
        buf,
        "tts:fontSelectionStrategy",
        &attrs.tts_font_selection_strategy,
    );
    serialize_opt_attr(buf, "tts:fontShear", &attrs.tts_font_shear);
    serialize_opt_attr(buf, "tts:fontSize", &attrs.tts_font_size);
    serialize_opt_attr(buf, "tts:fontStyle", &attrs.tts_font_style);
    serialize_opt_attr(buf, "tts:fontVariant", &attrs.tts_font_variant);
    serialize_opt_attr(buf, "tts:fontWeight", &attrs.tts_font_weight);
    serialize_opt_attr(buf, "tts:ipd", &attrs.tts_ipd);
    serialize_opt_attr(buf, "tts:letterSpacing", &attrs.tts_letter_spacing);
    serialize_opt_attr(buf, "tts:lineHeight", &attrs.tts_line_height);
    serialize_opt_attr(buf, "tts:lineShear", &attrs.tts_line_shear);
    serialize_opt_attr(buf, "tts:luminanceGain", &attrs.tts_luminance_gain);
    serialize_opt_attr(buf, "tts:opacity", &attrs.tts_opacity);
    serialize_opt_attr(buf, "tts:origin", &attrs.tts_origin);
    serialize_opt_attr(buf, "tts:overflow", &attrs.tts_overflow);
    serialize_opt_attr(buf, "tts:padding", &attrs.tts_padding);
    serialize_opt_attr(buf, "tts:position", &attrs.tts_position);
    serialize_opt_attr(buf, "tts:ruby", &attrs.tts_ruby);
    serialize_opt_attr(buf, "tts:rubyAlign", &attrs.tts_ruby_align);
    serialize_opt_attr(buf, "tts:rubyPosition", &attrs.tts_ruby_position);
    serialize_opt_attr(buf, "tts:rubyReserve", &attrs.tts_ruby_reserve);
    serialize_opt_attr(buf, "tts:shear", &attrs.tts_shear);
    serialize_opt_attr(buf, "tts:showBackground", &attrs.tts_show_background);
    serialize_opt_attr(buf, "tts:textAlign", &attrs.tts_text_align);
    serialize_opt_attr(buf, "tts:textCombine", &attrs.tts_text_combine);
    serialize_opt_attr(buf, "tts:textDecoration", &attrs.tts_text_decoration);
    serialize_opt_attr(buf, "tts:textEmphasis", &attrs.tts_text_emphasis);
    serialize_opt_attr(buf, "tts:textOrientation", &attrs.tts_text_orientation);
    serialize_opt_attr(buf, "tts:textOutline", &attrs.tts_text_outline);
    serialize_opt_attr(buf, "tts:textShadow", &attrs.tts_text_shadow);
    serialize_opt_attr(buf, "tts:unicodeBidi", &attrs.tts_unicode_bidi);
    serialize_opt_attr(buf, "tts:visibility", &attrs.tts_visibility);
    serialize_opt_attr(buf, "tts:wrapOption", &attrs.tts_wrap_option);
    serialize_opt_attr(buf, "tts:writingMode", &attrs.tts_writing_mode);
    serialize_opt_attr(buf, "tts:zIndex", &attrs.tts_z_index);
    serialize_opt_attr(buf, "tta:gain", &attrs.tta_gain);
    serialize_opt_attr(buf, "tta:pan", &attrs.tta_pan);
    serialize_opt_attr(buf, "tta:pitch", &attrs.tta_pitch);
    serialize_opt_attr(buf, "tta:speak", &attrs.tta_speak);
    serialize_opt_attr(buf, "itts:forcedDisplay", &attrs.itts_forced_display);
    serialize_opt_attr(buf, "itts:fillLineGap", &attrs.itts_fill_line_gap);
    serialize_opt_attr(buf, "ebutts:linePadding", &attrs.ebutts_line_padding);
    serialize_opt_attr(buf, "ebutts:multiRowAlign", &attrs.ebutts_multi_row_align);
}

/// Basic XML escaping for attribute values and text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ─── Style attribute parsing ──────────────────────────────────────

/// Parse all TT Style attributes (and IMSC/EBU style extensions) from an element node.
#[allow(clippy::too_many_lines)]
fn parse_style_attributes(node: roxmltree::Node<'_, '_>) -> StyleAttributes {
    StyleAttributes {
        tts_background_color: attribute_value(&node, NS_TTS, "backgroundColor")
            .map(|s| s.to_string()),
        tts_background_clip: attribute_value(&node, NS_TTS, "backgroundClip")
            .map(|s| s.to_string()),
        tts_background_extent: attribute_value(&node, NS_TTS, "backgroundExtent")
            .map(|s| s.to_string()),
        tts_background_image: attribute_value(&node, NS_TTS, "backgroundImage")
            .map(|s| s.to_string()),
        tts_background_origin: attribute_value(&node, NS_TTS, "backgroundOrigin")
            .map(|s| s.to_string()),
        tts_background_position: attribute_value(&node, NS_TTS, "backgroundPosition")
            .map(|s| s.to_string()),
        tts_background_repeat: attribute_value(&node, NS_TTS, "backgroundRepeat")
            .map(|s| s.to_string()),
        tts_border: attribute_value(&node, NS_TTS, "border").map(|s| s.to_string()),
        tts_bpd: attribute_value(&node, NS_TTS, "bpd").map(|s| s.to_string()),
        tts_color: attribute_value(&node, NS_TTS, "color").map(|s| s.to_string()),
        tts_direction: attribute_value(&node, NS_TTS, "direction").map(|s| s.to_string()),
        tts_disparity: attribute_value(&node, NS_TTS, "disparity").map(|s| s.to_string()),
        tts_display: attribute_value(&node, NS_TTS, "display").map(|s| s.to_string()),
        tts_display_align: attribute_value(&node, NS_TTS, "displayAlign").map(|s| s.to_string()),
        tts_extent: attribute_value(&node, NS_TTS, "extent").map(|s| s.to_string()),
        tts_font_family: attribute_value(&node, NS_TTS, "fontFamily").map(|s| s.to_string()),
        tts_font_kerning: attribute_value(&node, NS_TTS, "fontKerning").map(|s| s.to_string()),
        tts_font_selection_strategy: attribute_value(&node, NS_TTS, "fontSelectionStrategy")
            .map(|s| s.to_string()),
        tts_font_shear: attribute_value(&node, NS_TTS, "fontShear").map(|s| s.to_string()),
        tts_font_size: attribute_value(&node, NS_TTS, "fontSize").map(|s| s.to_string()),
        tts_font_style: attribute_value(&node, NS_TTS, "fontStyle").map(|s| s.to_string()),
        tts_font_variant: attribute_value(&node, NS_TTS, "fontVariant").map(|s| s.to_string()),
        tts_font_weight: attribute_value(&node, NS_TTS, "fontWeight").map(|s| s.to_string()),
        tts_ipd: attribute_value(&node, NS_TTS, "ipd").map(|s| s.to_string()),
        tts_letter_spacing: attribute_value(&node, NS_TTS, "letterSpacing").map(|s| s.to_string()),
        tts_line_height: attribute_value(&node, NS_TTS, "lineHeight").map(|s| s.to_string()),
        tts_line_shear: attribute_value(&node, NS_TTS, "lineShear").map(|s| s.to_string()),
        tts_luminance_gain: attribute_value(&node, NS_TTS, "luminanceGain").map(|s| s.to_string()),
        tts_opacity: attribute_value(&node, NS_TTS, "opacity").map(|s| s.to_string()),
        tts_origin: attribute_value(&node, NS_TTS, "origin").map(|s| s.to_string()),
        tts_overflow: attribute_value(&node, NS_TTS, "overflow").map(|s| s.to_string()),
        tts_padding: attribute_value(&node, NS_TTS, "padding").map(|s| s.to_string()),
        tts_position: attribute_value(&node, NS_TTS, "position").map(|s| s.to_string()),
        tts_ruby: attribute_value(&node, NS_TTS, "ruby").map(|s| s.to_string()),
        tts_ruby_align: attribute_value(&node, NS_TTS, "rubyAlign").map(|s| s.to_string()),
        tts_ruby_position: attribute_value(&node, NS_TTS, "rubyPosition").map(|s| s.to_string()),
        tts_ruby_reserve: attribute_value(&node, NS_TTS, "rubyReserve").map(|s| s.to_string()),
        tts_shear: attribute_value(&node, NS_TTS, "shear").map(|s| s.to_string()),
        tts_show_background: attribute_value(&node, NS_TTS, "showBackground")
            .map(|s| s.to_string()),
        tts_text_align: attribute_value(&node, NS_TTS, "textAlign").map(|s| s.to_string()),
        tts_text_combine: attribute_value(&node, NS_TTS, "textCombine").map(|s| s.to_string()),
        tts_text_decoration: attribute_value(&node, NS_TTS, "textDecoration")
            .map(|s| s.to_string()),
        tts_text_emphasis: attribute_value(&node, NS_TTS, "textEmphasis").map(|s| s.to_string()),
        tts_text_orientation: attribute_value(&node, NS_TTS, "textOrientation")
            .map(|s| s.to_string()),
        tts_text_outline: attribute_value(&node, NS_TTS, "textOutline").map(|s| s.to_string()),
        tts_text_shadow: attribute_value(&node, NS_TTS, "textShadow").map(|s| s.to_string()),
        tts_unicode_bidi: attribute_value(&node, NS_TTS, "unicodeBidi").map(|s| s.to_string()),
        tts_visibility: attribute_value(&node, NS_TTS, "visibility").map(|s| s.to_string()),
        tts_wrap_option: attribute_value(&node, NS_TTS, "wrapOption").map(|s| s.to_string()),
        tts_writing_mode: attribute_value(&node, NS_TTS, "writingMode").map(|s| s.to_string()),
        tts_z_index: attribute_value(&node, NS_TTS, "zIndex").map(|s| s.to_string()),
        tta_gain: attribute_value(&node, NS_TTA, "gain").map(|s| s.to_string()),
        tta_pan: attribute_value(&node, NS_TTA, "pan").map(|s| s.to_string()),
        tta_pitch: attribute_value(&node, NS_TTA, "pitch").map(|s| s.to_string()),
        tta_speak: attribute_value(&node, NS_TTA, "speak").map(|s| s.to_string()),
        itts_forced_display: attribute_value(&node, NS_ITTS, "forcedDisplay")
            .map(|s| s.to_string()),
        itts_fill_line_gap: attribute_value(&node, NS_ITTS, "fillLineGap").map(|s| s.to_string()),
        ebutts_line_padding: attribute_value(&node, NS_EBUTTS, "linePadding")
            .map(|s| s.to_string()),
        ebutts_multi_row_align: attribute_value(&node, NS_EBUTTS, "multiRowAlign")
            .map(|s| s.to_string()),
    }
}
