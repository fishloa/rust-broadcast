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

use crate::error::{Error, Result};
use crate::foreign::{ForeignAttribute, UnknownElement, UnknownNode};

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
/// TT Extension namespace: `http://www.w3.org/ns/ttml/extension/`
pub const NS_TT_EXTENSION: &str = "http://www.w3.org/ns/ttml/extension/";
/// TT Resource namespace: `http://www.w3.org/ns/ttml/resource/`
pub const NS_TT_RESOURCE: &str = "http://www.w3.org/ns/ttml/resource/";
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
/// The XML namespace-declaration prefix URI (XML Names 1.0). Reserving a
/// prefix or URI here in `add_scoped` is defensive: roxmltree never reports
/// `xmlns` itself as a namespace, but a struct-built `ForeignAttribute`
/// naming it must not corrupt the `<tt>` declarations (#1110/TT-W1).
pub const NS_XMLNS: &str = "http://www.w3.org/2000/xmlns/";
/// XLink namespace (the `image` element's `xlink:*` attributes — TTML2 §9.1.5).
pub const NS_XLINK: &str = "http://www.w3.org/1999/xlink";

// ─── Nesting limits ────────────────────────────────────────────────

/// Maximum nesting depth for span/metadata elements to prevent stack overflow.
const MAX_NESTING_DEPTH: usize = 64;

/// Maximum depth of an [`UnknownElement`] subtree captured by the parser
/// (TTML2 §7.2 foreign content); the subtree shape is input-controlled, so
/// it is bounded at parse time just like span/metadata nesting.
const MAX_UNKNOWN_DEPTH: usize = 64;

/// Prefix generated for a preserved namespace whose original `xmlns:`
/// prefix is already owned by a different URI (#1110/TT-W1).
const FALLBACK_PREFIX: &str = "ttmfallback";

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

    /// Serialize this document to an XML string. Takes `&mut self` because
    /// the namespace collection pass normalizes preserved foreign bindings
    /// (an inner-scope `xmlns:` override moves to `<tt>`, and preserved items
    /// in its old scope are re-pointed at the winning URI) so the output
    /// re-parses with exactly the namespaces the original resolved to
    /// (#1110/TT-W1). The document remains parseable/serializable afterwards.
    pub fn to_xml(&mut self) -> String {
        let mut buf = String::new();
        buf.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
        buf.push('\n');
        let mut ns = collect_namespaces(&mut self.tt);
        serialize_tt_element(&self.tt, &mut buf, 0, &mut ns);
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Optional `<head>` child.
    pub head: Option<HeadElement>,
    /// Optional `<body>` child.
    pub body: Option<BodyElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// Vendor `xmlns:` declarations written on `<tt>` itself. roxmltree
    /// consumes every namespace declaration, so a vendor binding that only a
    /// descendant's foreign attribute uses would otherwise be lost; capturing
    /// it here lets the serializer re-declare it (TTML2 §7.2, #1110/TT-W1).
    /// Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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

        // §7.2.11: default, if a frame rate IS SPECIFIED (`ttp:frameRate`
        // present — not merely defaulted to 30), is the *effective* frame
        // rate (`frame_rate * frameRateMultiplier`) times `subFrameRate`;
        // else 1 tick/second (#1108/TT-W4: this used to always multiply the
        // possibly-defaulted `frame_rate` by `sub_frame_rate`, giving 30
        // instead of 1 when no frame rate was specified at all, and via an
        // unchecked `u32` multiply that panics in debug / wraps in release
        // on a maliciously large `ttp:frameRate`/`ttp:subFrameRate`).
        let tick_rate: u32 = self
            .ttp_tick_rate
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| {
                if self.ttp_frame_rate.is_none() {
                    return 1;
                }
                u64::from(frame_rate)
                    .saturating_mul(u64::from(frame_mult_num))
                    .checked_div(u64::from(frame_mult_den))
                    .unwrap_or(0)
                    .saturating_mul(u64::from(sub_frame_rate))
                    .min(u64::from(u32::MAX)) as u32
            });

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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Styling section.
    pub styling: Option<StylingElement>,
    /// Layout section.
    pub layout: Option<LayoutElement>,
    /// `<resources>` container (TTML2 §9.1.6).
    pub resources: Option<ResourcesElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child `<div>` elements.
    pub divs: Vec<DivElement>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// SMPTE-TT `smpte:backgroundImage` attribute (IMSC 1.1 §9.4.5).
    pub smpte_background_image: Option<String>,
    /// Child `<audio>` elements (TTML2 §9.1.1, Embedded.class).
    pub audio: Vec<AudioElement>,
    /// Child `<p>` elements.
    pub paragraphs: Vec<PElement>,
    /// Child `<image>` elements.
    pub images: Vec<ImageElement>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child content: text nodes, `<span>`, `<br>`, `<image>`, `<audio>`
    /// (Embedded.class is legal in `<p>` per TTML2 §8.1.5).
    pub content: Vec<InlineContent>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child content: text nodes, nested `<span>`, `<br>`, `<image>`, `<audio>`
    /// (TTML2 §8.1.6 allows Embedded.class in `<span>` too).
    pub content: Vec<InlineContent>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `ttm:role` — TTML2 §14.6.1 (§8.1.7 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// `ttm:role` — TTML2 §14.6.1.
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child `<metadata>` elements (TTML2 §13.1.3 content model).
    pub metadata: Vec<MetadataChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// `ttm:role` — TTML2 §14.6.1 (§9.1.5 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// `src` URI.
    pub src: Option<String>,
    /// `type` MIME type.
    pub type_: Option<String>,
    /// `tts:extent` on the image.
    pub tts_extent: Option<String>,
    /// `xlink:href` (TTML2 §9.1.5).
    pub xlink_href: Option<String>,
    /// `xlink:role` (TTML2 §9.1.5).
    pub xlink_role: Option<String>,
    /// `xlink:arcrole` (TTML2 §9.1.5).
    pub xlink_arcrole: Option<String>,
    /// `xlink:title` (TTML2 §9.1.5).
    pub xlink_title: Option<String>,
    /// `xlink:show` (TTML2 §9.1.5, default "new").
    pub xlink_show: Option<String>,
    /// Other style attributes.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Child `<set>` animation elements (§9.1.5 content model).
    pub animations: Vec<AnimationChild>,
    /// Child `<source>` elements (TTML2 §9.1.7).
    pub sources: Vec<SourceElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// An `<image>` element (Embedded.class — TTML2 §9.1.5, legal in `<p>`/`<span>`).
    Image(Box<ImageElement>),
    /// An `<audio>` element (Embedded.class — TTML2 §9.1.1, legal in `<p>`/`<span>`).
    Audio(Box<AudioElement>),
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
    /// An unmodeled metadata-namespace element, preserved losslessly
    /// (TTML2 §7.2, #1110/TT-W1).
    Unknown(UnknownElement),
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child metadata items.
    pub children: Vec<MetadataChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// Text content.
    pub text: String,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition`.
    pub condition: Option<String>,
    /// `type`: person, character, group, organization, other.
    pub type_: Option<String>,
    /// Child `<ttm:name>` elements.
    pub names: Vec<TtmNameElement>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition`.
    pub condition: Option<String>,
    /// `type`: full, family, given, alias, other.
    pub type_: Option<String>,
    /// Text content.
    pub text: String,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition`.
    pub condition: Option<String>,
    /// `name` — either a named-item or QName.
    pub name: Option<String>,
    /// Text content.
    pub text: Option<String>,
    /// Nested `<ttm:item>` elements.
    pub items: Vec<TtmItemElement>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
}

/// Generic EBU-TT-M element (like `<ebuttm:documentMetadata>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EbuttmElement {
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Children within the EBU-TT-M element.
    pub children: Vec<MetadataChild>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
}

/// Text-only EBU-TT-M element (like `<ebuttm:conformsToStandard>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct EbuttmTextElement {
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Text content.
    pub text: String,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
    /// Text content.
    pub text: String,
    /// `xmlns:` declarations scoped to this element; re-emitted on `<tt>`
    /// (TTML2 §7.2, #1110/TT-W1). Always set by `parse_str`.
    pub scoped_namespaces: Option<Vec<(String, String)>>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
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
    /// `ttm:role` — TTML2 §14.6.1.
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Style attributes on the region.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Child `<metadata>` elements (§11.1.2 content model).
    pub metadata: Vec<MetadataChild>,
    /// Child `<set>` animation elements (§11.1.2 content model).
    pub animations: Vec<AnimationChild>,
    /// Child `<style>` elements (§11.1.2 content model).
    pub styles: Vec<StyleElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `<initial>` elements.
    pub initials: Vec<InitialElement>,
    /// `<style>` elements.
    pub styles: Vec<StyleElement>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition`.
    pub condition: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition`.
    pub condition: Option<String>,
    /// `style` (IDREFS, for chaining).
    pub style: Option<String>,
    /// Style attributes.
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
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

// ─── Embedded content elements (TTML2 §9.1) ───────────────────────

/// An `<audio>` element — TTML2 §9.1.1.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct AudioElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `begin` time expression.
    pub begin: Option<String>,
    /// `dur` time expression.
    pub dur: Option<String>,
    /// `end` time expression.
    pub end: Option<String>,
    /// `clipBegin` time expression (§9.1.1).
    pub clip_begin: Option<String>,
    /// `clipEnd` time expression (§9.1.1).
    pub clip_end: Option<String>,
    /// `timeContainer`: "par" or "seq".
    pub time_container: Option<String>,
    /// `region` IDREF.
    pub region: Option<String>,
    /// `style` IDREFS.
    pub style: Option<String>,
    /// `animate` IDREFS.
    pub animate: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `src` URI (§9.1.1).
    pub src: Option<String>,
    /// `type` MIME type (§9.1.1).
    pub type_: Option<String>,
    /// `ttm:role` — TTML2 §14.6.1 (§9.1.1 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Style attributes (§9.1.1 allows any TT Style Namespace attributes).
    pub style_attributes: StyleAttributes,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Animation children.
    pub animations: Vec<AnimationChild>,
    /// Child `<source>` elements (§9.1.7).
    pub sources: Vec<SourceElement>,

    /// Character data directly inside `<audio>` (TTML2 §9.3 allows
    /// Character.class content; IMSC audio is usually empty).
    pub text: Option<String>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
}

/// A `<chunk>` element — TTML2 §9.1.2 (base64-encoded fragment of an
/// embedded `data` resource).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ChunkElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `encoding`: base16/base32/base32hex/base64/base64url (§9.1.2).
    pub encoding: Option<String>,
    /// `length` (xsd:nonNegativeInteger).
    pub length: Option<String>,
    /// `xml:base` — TTML2 §7.4 (a `<chunk>` carries only this xml:* attr).
    pub xml_base: Option<String>,
    /// Character data of the chunk.
    pub text: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
}

/// A `<data>` element — TTML2 §9.1.3 (an embedded binary resource, inline,
/// chunked, or by reference via `source`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct DataElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `encoding`: base16/base32/base32hex/base64/base64url (§9.1.3).
    pub encoding: Option<String>,
    /// `format` (data format — NCName or URI).
    pub format: Option<String>,
    /// `length` (xsd:nonNegativeInteger).
    pub length: Option<String>,
    /// `src` URI.
    pub src: Option<String>,
    /// `type` MIME type.
    pub type_: Option<String>,
    /// `ttm:role` — TTML2 §14.6.1 (§9.1.3 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Character data (when content is #PCDATA).
    pub text: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Child `<chunk>` elements (when content is `(Metadata.class*, chunk+)`).
    pub chunks: Vec<ChunkElement>,
    /// Child `<source>` elements (when content is `(Metadata.class*, source+)`).
    pub sources: Vec<SourceElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
}

/// A `<font>` element — TTML2 §9.1.4 (only legal inside `<resources>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct FontElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `family` name.
    pub family: Option<String>,
    /// `range` (unicode range).
    pub range: Option<String>,
    /// `style`: normal/italic/oblique (§9.1.4 font style — the `style`
    /// attribute of `<font>` is this value, not an IDREFS binding).
    pub style_: Option<String>,
    /// `src` URI.
    pub src: Option<String>,
    /// `type` MIME type.
    pub type_: Option<String>,
    /// `weight`: normal/bold (§9.1.4).
    pub weight: Option<String>,
    /// `ttm:role` — TTML2 §14.6.1 (§9.1.4 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Child `<set>` animation elements (§9.1.5 content model).
    pub animations: Vec<AnimationChild>,
    /// Child `<source>` elements (§9.1.7).
    pub sources: Vec<SourceElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
}

/// A `<resources>` container — TTML2 §9.1.6 (top-level inside `<head>`).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct ResourcesElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Child `<data>` elements (§9.1.3).
    pub data: Vec<DataElement>,
    /// Child `<image>` elements (§9.1.5).
    pub images: Vec<ImageElement>,
    /// Child `<audio>` elements (§9.1.1).
    pub audio: Vec<AudioElement>,
    /// Child `<font>` elements (§9.1.4).
    pub fonts: Vec<FontElement>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
}

/// A `<source>` element — TTML2 §9.1.7 (reference to an out-of-line or
/// alternative resource).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct SourceElement {
    /// XML id.
    pub xml_id: Option<String>,
    /// XML language.
    pub xml_lang: Option<String>,
    /// XML space.
    pub xml_space: Option<XmlSpace>,
    /// `xml:base` — TTML2 §7.4.
    pub xml_base: Option<String>,
    /// `condition` expression.
    pub condition: Option<String>,
    /// `format` (data format — NCName or URI).
    pub format: Option<String>,
    /// `src` URI.
    pub src: Option<String>,
    /// `type` MIME type.
    pub type_: Option<String>,
    /// `ttm:role` — TTML2 §14.6.1 (§9.1.7 allows TT Metadata Namespace attributes).
    pub ttm_role: Option<String>,
    /// `ttm:roleSource` — TTML2 §14.6.2.
    pub ttm_role_source: Option<String>,
    /// Attributes in namespaces this crate does not model, preserved with
    /// their original `xmlns:` prefix bindings (TTML2 §7.2, #1110/TT-W1).
    pub foreign_attributes: Vec<ForeignAttribute>,
    /// Metadata children.
    pub metadata: Vec<MetadataChild>,
    /// Optional nested `<data>` (§9.1.7 content model: `Metadata.class*, data?`).
    pub data: Option<Box<DataElement>>,
    /// Unmodeled child elements, in document order relative to each other (their
    /// position among the modeled/metadata/animation children is not tracked;
    /// TTML2 §7.2/§7.3,
    /// #1110/TT-W1).
    pub unknown_children: Vec<UnknownElement>,
}

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

/// Namespaces whose attributes the crate models explicitly: an attribute in
/// one of these is parsed into a named field (or, for a name the crate does
/// not model, belongs to the element's own vocabulary and is *not* foreign).
/// Everything else is captured as a [`ForeignAttribute`] (TTML2 §7.2).
const MODELED_ATTRIBUTE_NAMESPACES: &[&str] = &[
    NS_TT,
    NS_TTP,
    NS_TTS,
    NS_TTA,
    NS_TTM,
    NS_TT_PROFILE,
    NS_TT_FEATURE,
    NS_TT_EXTENSION,
    NS_TT_RESOURCE,
    NS_ITTS,
    NS_ITTP,
    NS_ITTM,
    NS_EBUTTS,
    NS_EBUTTM,
    NS_SMPTE,
    NS_XML,
    NS_XLINK,
    "", // no-namespace attributes like begin/dur/end
];

/// Capture every attribute of `node` whose namespace the crate does not
/// model, preserving the original `xmlns:` prefix that was in scope for it
/// (TTML2 §7.2, #1110/TT-W1).
fn foreign_attributes(node: &roxmltree::Node<'_, '_>) -> Vec<ForeignAttribute> {
    // Each preserved attribute also carries the `xmlns:` declarations in
    // scope: roxmltree consumes declarations, and an inner-scope override of
    // a prefix (e.g. `xmlns:v="urn:two"` written on the element itself) is
    // otherwise unrecoverable, so the serializer needs them to reproduce the
    // namespace each attribute resolved to (#1110/TT-W1).
    let scoped = capture_scoped_namespaces(node);
    node.attributes()
        .filter(|attr| !MODELED_ATTRIBUTE_NAMESPACES.contains(&attr.namespace().unwrap_or("")))
        .map(|attr| {
            let mut foreign = ForeignAttribute::new(
                prefix_for_uri(node, attr.namespace()),
                attr.namespace(),
                attr.name(),
                attr.value(),
            );
            foreign.scoped_namespaces = scoped.clone().unwrap_or_default();
            foreign
        })
        .collect()
}

/// The namespaced declarations on `node` that the crate does not already
/// re-declare on `<tt>`. roxmltree's `namespaces()` walks the whole scope
/// chain, so a declaration written on an ancestor would be captured again at
/// every descendant; declarations for URIs the crate always binds on `<tt>`
/// (the five core bindings) and for its conventional extension prefixes are
/// filtered out — a vendor namespace declared on `<tt>` therefore stays on
/// `<tt>` and is *not* copied onto every child element (TTML2 §7.2,
/// #1110/TT-W1). What remains is a declaration local to this element (or a
/// vendor binding from further up that only unknown content uses), which is
/// mirrored to `<tt>` on output: namespace-equivalent, since widening a
/// scope changes no resolved name.
fn capture_scoped_namespaces(node: &roxmltree::Node<'_, '_>) -> Option<Vec<(String, String)>> {
    let decls: Vec<(String, String)> = node
        .namespaces()
        .filter_map(|n| n.name().map(|p| (String::from(p), String::from(n.uri()))))
        .filter(|(p, u)| {
            !p.is_empty()
                && p != "xml"
                && p != "xmlns"
                && !KEY_BINDINGS.iter().any(|(_, ku)| ku == u)
                && !KNOWN_FOREIGN_NAMESPACES.iter().any(|(_, ku)| ku == u)
        })
        .collect();
    if decls.is_empty() { None } else { Some(decls) }
}

/// Find the `xmlns:` prefix bound to `uri` in scope of `node` (`None` for
/// no namespace or the default-namespace binding, which never carries an
/// attribute). roxmltree lists the default namespace first in
/// `namespaces()`; the first non-default prefix matching the URI wins.
fn prefix_for_uri<'a>(node: &roxmltree::Node<'_, 'a>, uri: Option<&str>) -> Option<&'a str> {
    let uri = uri?;
    node.namespaces().find_map(|n| {
        let name = n.name()?;
        (n.uri() == uri && !name.is_empty()).then_some(name)
    })
}

/// Capture an unmodeled element as a lossless [`UnknownElement`] subtree:
/// original prefix, namespace URI, all attributes (each with the prefix
/// that was in scope for it), child elements and merged text, in document
/// order (TTML2 §7.2/§7.3, #1110/TT-W1).
///
/// roxmltree resolves QNames and consumes `xmlns:` declarations, so the
/// subtree is lossless in *resolved* identity (URI + local name + value)
/// and remembers the original prefix spelling via `prefix`; the
/// serializer re-declares every preserved URI on `<tt>` (original prefix
/// when still free, otherwise a generated `ttmfallbackN`), which is
/// namespace-equivalent to the original document.
fn parse_unknown_element(
    node: &roxmltree::Node<'_, '_>,
    depth: usize,
) -> Result<Box<UnknownElement>> {
    if depth >= MAX_UNKNOWN_DEPTH {
        return Err(Error::ConstraintViolation {
            constraint: "Unknown element nesting depth limit".to_string(),
            detail: "Foreign element nesting exceeds maximum depth of 64".to_string(),
        });
    }

    let attributes: Vec<ForeignAttribute> = node
        .attributes()
        .map(|attr| {
            ForeignAttribute::new(
                prefix_for_uri(node, attr.namespace()),
                attr.namespace(),
                attr.name(),
                attr.value(),
            )
        })
        .collect();

    let mut children = Vec::new();
    for child in node.children() {
        match child.node_type() {
            roxmltree::NodeType::Element => {
                children.push(UnknownNode::Element(parse_unknown_element(
                    &child,
                    depth + 1,
                )?));
            }
            roxmltree::NodeType::Text => {
                let text = child.text().unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                // roxmltree splits text nodes at entity references; merge
                // adjacent text so the re-serialized subtree is stable.
                if let Some(UnknownNode::Text(last)) = children.last_mut() {
                    last.push_str(text);
                } else {
                    children.push(UnknownNode::Text(String::from(text)));
                }
            }
            // Comments and PIs are not representable (never serialized).
            _ => {}
        }
    }

    // roxmltree resolves element QNames to (uri, local) and does not expose
    // the original element prefix; recover the first non-default prefix
    // bound to the element's namespace in scope, which is the prefix the
    // author must have written for a foreign element.
    let element_ns = node.tag_name().namespace();
    // roxmltree lists in-scope declarations innermost-first, so the *first*
    // non-empty prefix bound to the element's URI is the one this element
    // itself used (a local `xmlns:v` override beats an ancestor's binding).
    let prefix = node
        .namespaces()
        .find(|n| n.name().is_some_and(|nm| !nm.is_empty()) && Some(n.uri()) == element_ns)
        .and_then(|n| n.name())
        .map(String::from);
    Ok(Box::new(UnknownElement {
        prefix,
        namespace: node.tag_name().namespace().map(String::from),
        local_name: String::from(node.tag_name().name()),
        attributes,
        children,
        scoped_namespaces: capture_scoped_namespaces(node),
    }))
}

// ─── XML Parsing Functions ─────────────────────────────────────────

/// Parse the root `<tt>` element from a roxmltree node.
fn parse_tt_element(node: roxmltree::Node<'_, '_>) -> Result<TtElement> {
    let mut head = None;
    let mut body = None;
    let mut unknown_children = Vec::new();

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
            // Foreign/unrecognized elements are kept losslessly (§7.2/§7.3).
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    Ok(TtElement {
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
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
        foreign_attributes: foreign_attributes(&node),
        head,
        body,
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_head_element(node: roxmltree::Node<'_, '_>) -> Result<HeadElement> {
    let mut metadata = Vec::new();
    let mut styling = None;
    let mut layout = None;
    let mut resources = None;
    let mut unknown_children = Vec::new();

    // Also collect metadata from top-level
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
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
            ("resources", Some(NS_TT)) => {
                resources = Some(parse_resources_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    Ok(HeadElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        metadata,
        styling,
        layout,
        resources,
        unknown_children,
    })
}

fn parse_body_element(node: roxmltree::Node<'_, '_>) -> Result<BodyElement> {
    let mut divs = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut unknown_children = Vec::new();

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
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
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
                animations.push(AnimationChild::Set(*parse_set_element(child)?));
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(BodyElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        divs,
        metadata,
        animations,
        unknown_children,
    })
}

fn parse_div_element(node: roxmltree::Node<'_, '_>) -> Result<DivElement> {
    let mut paragraphs = Vec::new();
    let mut images = Vec::new();
    let mut audio = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut unknown_children = Vec::new();

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
                images.push(*parse_image_element(child)?);
            }
            ("audio", Some(NS_TT)) => {
                audio.push(*parse_audio_element(child)?);
            }
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(*parse_set_element(child)?));
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(DivElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        smpte_background_image: attribute_value(&node, NS_SMPTE, "backgroundImage")
            .map(|s| s.to_string()),
        paragraphs,
        images,
        audio,
        metadata,
        animations,
        unknown_children,
    })
}

fn parse_p_element(node: roxmltree::Node<'_, '_>) -> Result<PElement> {
    let mut content: Vec<InlineContent> = Vec::new();
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut unknown_children = Vec::new();

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
                    content.push(InlineContent::Span(parse_span_element(child)?));
                }
                ("br", Some(NS_TT)) => {
                    content.push(InlineContent::Br(Box::new(parse_br_element(child)?)));
                }
                ("image", Some(NS_TT)) => {
                    content.push(InlineContent::Image(parse_image_element(child)?));
                }
                ("audio", Some(NS_TT)) => {
                    content.push(InlineContent::Audio(parse_audio_element(child)?));
                }
                ("metadata", Some(NS_TT)) => {
                    metadata.push(MetadataChild::Metadata(*parse_metadata_element_impl(
                        child, 0,
                    )?));
                }
                ("set", Some(NS_TT)) => {
                    animations.push(AnimationChild::Set(*parse_set_element(child)?));
                }
                _ => {
                    unknown_children.push(*parse_unknown_element(&child, 0)?);
                }
            }
        }
    }

    let style_attrs = parse_style_attributes(node);

    Ok(PElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        content,
        metadata,
        animations,
        unknown_children,
    })
}

fn parse_span_element(node: roxmltree::Node<'_, '_>) -> Result<Box<SpanElement>> {
    // The nested-content parse is boxed one level deep (see
    // `parse_span_element_impl`'s own note): each recursion level returns a
    // ~1.8 KB struct, and the 64 levels this parser allows would otherwise
    // put ~115 KB of return slots on the stack (#1110).
    parse_span_element_impl(node, 0)
}

fn parse_span_element_impl(
    node: roxmltree::Node<'_, '_>,
    depth: usize,
) -> Result<Box<SpanElement>> {
    // The child walk is iterative (explicit stack): `SpanElement` is ~1.8 KB
    // and the child *build* must live in the parent's frame, so ~64 levels of
    // recursion would put ~115 KB of intermediates on the stack and abort a
    // 2 MB test thread — one level deeper than `MAX_NESTING_DEPTH` allows
    // (#1110). The public API stays `Vec<InlineContent>`; only the walk uses
    // boxed frames.
    if depth >= MAX_NESTING_DEPTH {
        return Err(Error::ConstraintViolation {
            constraint: "Span nesting depth limit".to_string(),
            detail: "Span element nesting exceeds maximum depth of 64".to_string(),
        });
    }

    struct Frame {
        content: Vec<InlineContent>,
        metadata: Vec<MetadataChild>,
        animations: Vec<AnimationChild>,
        unknown_children: Vec<UnknownElement>,
    }

    enum Walk<'a> {
        Open(roxmltree::Node<'a, 'a>, usize),
        Text(roxmltree::Node<'a, 'a>),
        Inline(roxmltree::Node<'a, 'a>),
        Meta(roxmltree::Node<'a, 'a>),
        Close(roxmltree::Node<'a, 'a>),
    }

    let mut stack: Vec<Walk<'_>> = alloc::vec![Walk::Open(node, depth)];
    let mut open: Vec<Frame> = Vec::new();
    let mut root_span: Option<SpanElement> = None;
    while let Some(walk) = stack.pop() {
        match walk {
            Walk::Open(n, depth) => {
                if depth >= MAX_NESTING_DEPTH {
                    return Err(Error::ConstraintViolation {
                        constraint: "Span nesting depth limit".to_string(),
                        detail: "Span element nesting exceeds maximum depth of 64".to_string(),
                    });
                }
                open.push(Frame {
                    content: Vec::new(),
                    metadata: Vec::new(),
                    animations: Vec::new(),
                    unknown_children: Vec::new(),
                });
                stack.push(Walk::Close(n));
                for child in n.children().rev() {
                    if child.is_text() {
                        if !child.text().unwrap_or("").is_empty() {
                            stack.push(Walk::Text(child));
                        }
                    } else if child.is_element() {
                        let name = child.tag_name().name();
                        let ns = child.tag_name().namespace();
                        match (name, ns) {
                            ("span", Some(NS_TT)) => {
                                stack.push(Walk::Open(child, depth + 1));
                            }
                            ("br", Some(NS_TT))
                            | ("image", Some(NS_TT))
                            | ("audio", Some(NS_TT)) => {
                                stack.push(Walk::Inline(child));
                            }
                            ("metadata", Some(NS_TT)) | ("set", Some(NS_TT)) => {
                                stack.push(Walk::Meta(child));
                            }
                            _ => {
                                stack.push(Walk::Meta(child));
                            }
                        }
                    }
                }
            }
            Walk::Text(child) => {
                let Some(frame) = open.last_mut() else {
                    return Err(span_walk_invariant("text inside an open span"));
                };
                let text = child.text().unwrap_or("");
                if let Some(InlineContent::Text(last)) = frame.content.last_mut() {
                    last.push_str(text);
                } else {
                    frame.content.push(InlineContent::Text(text.to_string()));
                }
            }
            Walk::Inline(child) => {
                let Some(frame) = open.last_mut() else {
                    return Err(span_walk_invariant("inline child inside an open span"));
                };
                match (child.tag_name().name(), child.tag_name().namespace()) {
                    ("br", Some(NS_TT)) => frame
                        .content
                        .push(InlineContent::Br(Box::new(parse_br_element(child)?))),
                    ("image", Some(NS_TT)) => frame
                        .content
                        .push(InlineContent::Image(parse_image_element(child)?)),
                    ("audio", Some(NS_TT)) => frame
                        .content
                        .push(InlineContent::Audio(parse_audio_element(child)?)),
                    _ => {}
                }
            }
            Walk::Meta(child) => {
                let Some(frame) = open.last_mut() else {
                    return Err(span_walk_invariant("meta child inside an open span"));
                };
                let name = child.tag_name().name();
                let ns = child.tag_name().namespace();
                match (name, ns) {
                    ("metadata", Some(NS_TT)) => frame
                        .metadata
                        .push(MetadataChild::Metadata(*parse_metadata_element(child)?)),
                    ("set", Some(NS_TT)) => frame
                        .animations
                        .push(AnimationChild::Set(*parse_set_element(child)?)),
                    _ => frame
                        .unknown_children
                        .push(*parse_unknown_element(&child, 0)?),
                }
            }
            Walk::Close(n) => {
                let Some(frame) = open.pop() else {
                    return Err(span_walk_invariant("close without a matching open span"));
                };
                let style_attrs = parse_style_attributes(n);
                let span = SpanElement {
                    xml_id: attribute_value(&n, NS_XML, "id").map(|s| s.to_string()),
                    xml_lang: attribute_value(&n, NS_XML, "lang").map(|s| s.to_string()),
                    xml_space: parse_xml_space(attribute_value(&n, NS_XML, "space")),
                    xml_base: attribute_value(&n, NS_XML, "base").map(|s| s.to_string()),
                    begin: n.attribute("begin").map(|s| s.to_string()),
                    dur: n.attribute("dur").map(|s| s.to_string()),
                    end: n.attribute("end").map(|s| s.to_string()),
                    time_container: n.attribute("timeContainer").map(|s| s.to_string()),
                    region: n.attribute("region").map(|s| s.to_string()),
                    style: n.attribute("style").map(|s| s.to_string()),
                    animate: n.attribute("animate").map(|s| s.to_string()),
                    condition: n.attribute("condition").map(|s| s.to_string()),
                    style_attributes: style_attrs,
                    foreign_attributes: foreign_attributes(&n),
                    content: frame.content,
                    metadata: frame.metadata,
                    animations: frame.animations,
                    unknown_children: frame.unknown_children,
                };
                match open.last_mut() {
                    Some(parent) => parent.content.push(InlineContent::Span(Box::new(span))),
                    None => root_span = Some(span),
                }
            }
        }
    }
    root_span
        .map(Box::new)
        .ok_or_else(|| span_walk_invariant("the root span frame never closed"))
}

/// The iterative span walk's push/pop discipline was violated. Unreachable by
/// construction (every `Open` pushes its own `Close`), but reported as an
/// error rather than a panic so hostile input can never abort the parser.
fn span_walk_invariant(what: &'static str) -> Error {
    Error::ConstraintViolation {
        constraint: "Span walk invariant".to_string(),
        detail: what.to_string(),
    }
}
fn parse_br_element(node: roxmltree::Node<'_, '_>) -> Result<BrElement> {
    let style_attrs = parse_style_attributes(node);
    Ok(BrElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
    })
}

fn parse_set_element(node: roxmltree::Node<'_, '_>) -> Result<Box<SetElement>> {
    // Boxed return: `SetElement` embeds a full `StyleAttributes` (~1.8 KB),
    // and `<set>` recurses inside `<metadata>` up to `MAX_NESTING_DEPTH`
    // (#1110).
    let style_attrs = parse_style_attributes(node);
    let mut metadata = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if child.is_element()
            && child.tag_name().name() == "metadata"
            && child.tag_name().namespace() == Some(NS_TT)
        {
            metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
        } else if child.is_element() {
            unknown_children.push(*parse_unknown_element(&child, 0)?);
        }
    }
    Ok(Box::new(SetElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        fill: node.attribute("fill").map(|s| s.to_string()),
        repeat_count: node.attribute("repeatCount").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        metadata,
        unknown_children,
    }))
}

fn parse_image_element(node: roxmltree::Node<'_, '_>) -> Result<Box<ImageElement>> {
    // Boxed return (~2 KB struct): `<image>` is inline content, so it nests
    // with `<span>` up to `MAX_NESTING_DEPTH` (#1110).
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut sources = Vec::new();
    let mut unknown_children = Vec::new();
    parse_embedded_children(
        &node,
        &mut metadata,
        &mut animations,
        &mut sources,
        &mut unknown_children,
    )?;

    let style_attrs = parse_style_attributes(node);

    Ok(Box::new(ImageElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        tts_extent: attribute_value(&node, NS_TTS, "extent").map(|s| s.to_string()),
        xlink_href: attribute_value(&node, NS_XLINK, "href").map(|s| s.to_string()),
        xlink_role: attribute_value(&node, NS_XLINK, "role").map(|s| s.to_string()),
        xlink_arcrole: attribute_value(&node, NS_XLINK, "arcrole").map(|s| s.to_string()),
        xlink_title: attribute_value(&node, NS_XLINK, "title").map(|s| s.to_string()),
        xlink_show: attribute_value(&node, NS_XLINK, "show").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        metadata,
        animations,
        sources,
        unknown_children,
    }))
}

fn parse_metadata_element(node: roxmltree::Node<'_, '_>) -> Result<Box<MetadataElement>> {
    // The return value is boxed one level deep (see
    // `parse_span_element_impl`): the nesting this parser allows would put a
    // full `MetadataElement` return slot in every frame otherwise (#1110).
    parse_metadata_element_impl(node, 0)
}

/// Decide whether an element the crate models *structurally* (text-only or
/// fixed-shape metadata children) is actually well-formed; anything else —
/// foreign attributes, unmodeled child elements, or an element-only subtree
/// where only text was expected — falls back to lossless unknown capture so
/// the shape is preserved exactly (TTML2 §7.2, #1110/TT-W1).
fn is_pure_text_element(node: &roxmltree::Node<'_, '_>) -> bool {
    foreign_attributes(node).is_empty() && node.children().all(|c| !c.is_element())
}

/// Same check for a container element whose modeled children are known:
/// pass `children_ok` to say whether a child element is modeled.
fn is_pure_container<F: Fn(&roxmltree::Node<'_, '_>) -> bool>(
    node: &roxmltree::Node<'_, '_>,
    children_ok: F,
) -> bool {
    foreign_attributes(node).is_empty()
        && node.children().all(|c| !c.is_element() || children_ok(&c))
}

fn parse_metadata_element_impl(
    node: roxmltree::Node<'_, '_>,
    depth: usize,
) -> Result<Box<MetadataElement>> {
    if depth >= MAX_NESTING_DEPTH {
        return Err(Error::ConstraintViolation {
            constraint: "Metadata nesting depth limit".to_string(),
            detail: "Metadata element nesting exceeds maximum depth of 64".to_string(),
        });
    }

    let mut children = Vec::new();
    let mut unknown_children = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();

        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                // Moves the boxed child in; drop of a structurally deep
                // tree is a pre-existing concern (see the span drop note).
                children.push(MetadataChild::Metadata(*parse_metadata_element_impl(
                    child,
                    depth + 1,
                )?));
            }
            ("title", Some(NS_TTM)) if is_pure_text_element(&child) => {
                children.push(MetadataChild::TtmTitle(parse_ttm_text(child)?));
            }
            ("desc", Some(NS_TTM)) if is_pure_text_element(&child) => {
                children.push(MetadataChild::TtmDesc(parse_ttm_text(child)?));
            }
            ("copyright", Some(NS_TTM)) if is_pure_text_element(&child) => {
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
            ("documentMetadata", Some(NS_EBUTTM))
                if is_pure_container(&child, |c| {
                    c.tag_name().name() == "conformsToStandard"
                        && c.tag_name().namespace() == Some(NS_EBUTTM)
                        && is_pure_text_element(c)
                }) =>
            {
                children.push(MetadataChild::EbuttmDocumentMetadata(parse_ebuttm_element(
                    child,
                )?));
            }
            ("conformsToStandard", Some(NS_EBUTTM)) if is_pure_text_element(&child) => {
                children.push(MetadataChild::EbuttmConformsToStandard(parse_ebuttm_text(
                    child,
                )?));
            }
            ("altText", Some(NS_ITTM)) if is_pure_text_element(&child) => {
                children.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    Ok(Box::new(MetadataElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        children,
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    }))
}

fn parse_ttm_text(node: roxmltree::Node<'_, '_>) -> Result<TtmTextElement> {
    let text = node.text().unwrap_or("").to_string();
    let unknown_children = node
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_unknown_element(&c, 0).map(|u| *u))
        .collect::<Result<Vec<_>>>()?;
    Ok(TtmTextElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        text,
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ttm_agent(node: roxmltree::Node<'_, '_>) -> Result<TtmAgentElement> {
    let mut names = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if child.is_element()
            && child.tag_name().name() == "name"
            && child.tag_name().namespace() == Some(NS_TTM)
        {
            names.push(parse_ttm_name(child)?);
        } else if child.is_element() {
            unknown_children.push(*parse_unknown_element(&child, 0)?);
        }
    }
    Ok(TtmAgentElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        names,
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ttm_name(node: roxmltree::Node<'_, '_>) -> Result<TtmNameElement> {
    let text = node.text().unwrap_or("").to_string();
    let unknown_children = node
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_unknown_element(&c, 0).map(|u| *u))
        .collect::<Result<Vec<_>>>()?;
    Ok(TtmNameElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        text,
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ttm_item(node: roxmltree::Node<'_, '_>) -> Result<TtmItemElement> {
    let mut items = Vec::new();
    let mut text_parts = String::new();
    let mut unknown_children = Vec::new();

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
        } else if child.is_element() {
            unknown_children.push(*parse_unknown_element(&child, 0)?);
        }
    }

    Ok(TtmItemElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        name: node.attribute("name").map(|s| s.to_string()),
        text: if text_parts.is_empty() {
            None
        } else {
            Some(text_parts)
        },
        items,
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ittm_alt_text(node: roxmltree::Node<'_, '_>) -> Result<IttmAltTextElement> {
    let text = node.text().unwrap_or("").to_string();
    let unknown_children = node
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_unknown_element(&c, 0).map(|u| *u))
        .collect::<Result<Vec<_>>>()?;
    Ok(IttmAltTextElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        text,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ebuttm_element(node: roxmltree::Node<'_, '_>) -> Result<EbuttmElement> {
    let mut children = Vec::new();
    let mut unknown_children = Vec::new();
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
        } else {
            unknown_children.push(*parse_unknown_element(&child, 0)?);
        }
    }
    Ok(EbuttmElement {
        foreign_attributes: foreign_attributes(&node),
        children,
        unknown_children,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_ebuttm_text(node: roxmltree::Node<'_, '_>) -> Result<EbuttmTextElement> {
    let text = node.text().unwrap_or("").to_string();
    Ok(EbuttmTextElement {
        foreign_attributes: foreign_attributes(&node),
        text,
        scoped_namespaces: capture_scoped_namespaces(&node),
    })
}

fn parse_styling_element(node: roxmltree::Node<'_, '_>) -> Result<StylingElement> {
    let mut initials = Vec::new();
    let mut styles = Vec::new();
    let mut unknown_children = Vec::new();

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
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    Ok(StylingElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        initials,
        styles,
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
    })
}

fn parse_initial_element(node: roxmltree::Node<'_, '_>) -> Result<InitialElement> {
    let unknown_children = node
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_unknown_element(&c, 0).map(|u| *u))
        .collect::<Result<Vec<_>>>()?;
    Ok(InitialElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style_attributes: parse_style_attributes(node),
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
    })
}

fn parse_style_element(node: roxmltree::Node<'_, '_>) -> Result<StyleElement> {
    let unknown_children = node
        .children()
        .filter(|c| c.is_element())
        .map(|c| parse_unknown_element(&c, 0).map(|u| *u))
        .collect::<Result<Vec<_>>>()?;
    Ok(StyleElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        style_attributes: parse_style_attributes(node),
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
    })
}

fn parse_layout_element(node: roxmltree::Node<'_, '_>) -> Result<LayoutElement> {
    let mut regions = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if child.is_element()
            && child.tag_name().name() == "region"
            && child.tag_name().namespace() == Some(NS_TT)
        {
            regions.push(parse_region_element(child)?);
        } else if child.is_element() {
            unknown_children.push(*parse_unknown_element(&child, 0)?);
        }
    }

    Ok(LayoutElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        unknown_children,
        regions,
    })
}

fn parse_region_element(node: roxmltree::Node<'_, '_>) -> Result<RegionElement> {
    let style_attrs = parse_style_attributes(node);
    // §11.1.2 content: Metadata.class*, Animation.class*, style*
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut styles = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(*parse_set_element(child)?));
            }
            ("style", Some(NS_TT)) => {
                styles.push(parse_style_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }

    Ok(RegionElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        metadata,
        animations,
        styles,
        unknown_children,
    })
}

/// Shared child walk for `audio`/`image`: metadata, `set` animations,
/// `source` elements, and anything else preserved as an unknown subtree
/// (TTML2 §9.1 content model `Metadata.class*, Animation.class*, source*`).
fn parse_embedded_children(
    node: &roxmltree::Node<'_, '_>,
    metadata: &mut Vec<MetadataChild>,
    animations: &mut Vec<AnimationChild>,
    sources: &mut Vec<SourceElement>,
    unknown_children: &mut Vec<UnknownElement>,
) -> Result<()> {
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("altText", Some(NS_ITTM)) => {
                metadata.push(MetadataChild::IttmAltText(parse_ittm_alt_text(child)?));
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(*parse_set_element(child)?));
            }
            ("source", Some(NS_TT)) => {
                sources.push(parse_source_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }
    Ok(())
}

fn parse_audio_element(node: roxmltree::Node<'_, '_>) -> Result<Box<AudioElement>> {
    // Boxed return: `<audio>` is inline content and nests with `<span>` up
    // to `MAX_NESTING_DEPTH` (#1110).
    let mut text_parts = String::new();
    for child in node.children() {
        if child.is_text() {
            text_parts.push_str(child.text().unwrap_or(""));
        }
    }
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut sources = Vec::new();
    let mut unknown_children = Vec::new();
    parse_embedded_children(
        &node,
        &mut metadata,
        &mut animations,
        &mut sources,
        &mut unknown_children,
    )?;
    let style_attrs = parse_style_attributes(node);
    Ok(Box::new(AudioElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        begin: node.attribute("begin").map(|s| s.to_string()),
        dur: node.attribute("dur").map(|s| s.to_string()),
        end: node.attribute("end").map(|s| s.to_string()),
        clip_begin: node.attribute("clipBegin").map(|s| s.to_string()),
        clip_end: node.attribute("clipEnd").map(|s| s.to_string()),
        time_container: node.attribute("timeContainer").map(|s| s.to_string()),
        region: node.attribute("region").map(|s| s.to_string()),
        style: node.attribute("style").map(|s| s.to_string()),
        animate: node.attribute("animate").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        style_attributes: style_attrs,
        foreign_attributes: foreign_attributes(&node),
        metadata,
        animations,
        sources,
        text: if text_parts.trim().is_empty() {
            None
        } else {
            Some(text_parts)
        },
        unknown_children,
    }))
}

fn parse_chunk_element(node: roxmltree::Node<'_, '_>) -> Result<ChunkElement> {
    let text = node.text().unwrap_or("");
    Ok(ChunkElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        encoding: node.attribute("encoding").map(|s| s.to_string()),
        length: node.attribute("length").map(|s| s.to_string()),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        text: if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        },
        foreign_attributes: foreign_attributes(&node),
    })
}

fn parse_data_element(node: roxmltree::Node<'_, '_>) -> Result<Box<DataElement>> {
    // Boxed return: `<data>` recurses through `<source><data>…` up to
    // `MAX_NESTING_DEPTH` (#1110).
    let mut metadata = Vec::new();
    let mut chunks = Vec::new();
    let mut sources = Vec::new();
    let mut unknown_children = Vec::new();
    let mut text_parts = String::new();
    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                text_parts.push_str(t);
            }
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("chunk", Some(NS_TT)) => {
                chunks.push(parse_chunk_element(child)?);
            }
            ("source", Some(NS_TT)) => {
                sources.push(parse_source_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }
    Ok(Box::new(DataElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        encoding: node.attribute("encoding").map(|s| s.to_string()),
        format: node.attribute("format").map(|s| s.to_string()),
        length: node.attribute("length").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        text: if text_parts.trim().is_empty() {
            None
        } else {
            Some(text_parts)
        },
        foreign_attributes: foreign_attributes(&node),
        metadata,
        chunks,
        sources,
        unknown_children,
    }))
}

fn parse_font_element(node: roxmltree::Node<'_, '_>) -> Result<FontElement> {
    let mut metadata = Vec::new();
    let mut animations = Vec::new();
    let mut sources = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("source", Some(NS_TT)) => {
                sources.push(parse_source_element(child)?);
            }
            ("set", Some(NS_TT)) => {
                animations.push(AnimationChild::Set(*parse_set_element(child)?));
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }
    Ok(FontElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        family: node.attribute("family").map(|s| s.to_string()),
        range: node.attribute("range").map(|s| s.to_string()),
        style_: node.attribute("style").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        weight: node.attribute("weight").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        metadata,
        animations,
        sources,
        unknown_children,
    })
}

fn parse_resources_element(node: roxmltree::Node<'_, '_>) -> Result<ResourcesElement> {
    let mut metadata = Vec::new();
    let mut data = Vec::new();
    let mut images = Vec::new();
    let mut audio = Vec::new();
    let mut fonts = Vec::new();
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("data", Some(NS_TT)) => {
                data.push(*parse_data_element(child)?);
            }
            ("image", Some(NS_TT)) => {
                images.push(*parse_image_element(child)?);
            }
            ("audio", Some(NS_TT)) => {
                audio.push(*parse_audio_element(child)?);
            }
            ("font", Some(NS_TT)) => {
                fonts.push(parse_font_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }
    Ok(ResourcesElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        metadata,
        data,
        images,
        audio,
        fonts,
        unknown_children,
    })
}

fn parse_source_element(node: roxmltree::Node<'_, '_>) -> Result<SourceElement> {
    let mut metadata = Vec::new();
    let mut data = None;
    let mut unknown_children = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        let ns = child.tag_name().namespace();
        match (name, ns) {
            ("metadata", Some(NS_TT)) => {
                metadata.push(MetadataChild::Metadata(*parse_metadata_element(child)?));
            }
            ("data", Some(NS_TT)) => {
                data = Some(parse_data_element(child)?);
            }
            _ => {
                unknown_children.push(*parse_unknown_element(&child, 0)?);
            }
        }
    }
    Ok(SourceElement {
        xml_id: attribute_value(&node, NS_XML, "id").map(|s| s.to_string()),
        xml_lang: attribute_value(&node, NS_XML, "lang").map(|s| s.to_string()),
        xml_space: parse_xml_space(attribute_value(&node, NS_XML, "space")),
        xml_base: attribute_value(&node, NS_XML, "base").map(|s| s.to_string()),
        condition: node.attribute("condition").map(|s| s.to_string()),
        format: node.attribute("format").map(|s| s.to_string()),
        src: node.attribute("src").map(|s| s.to_string()),
        type_: node.attribute("type").map(|s| s.to_string()),
        ttm_role: attribute_value(&node, NS_TTM, "role").map(|s| s.to_string()),
        ttm_role_source: attribute_value(&node, NS_TTM, "roleSource").map(|s| s.to_string()),
        foreign_attributes: foreign_attributes(&node),
        metadata,
        data,
        unknown_children,
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

/// A document-wide `xmlns:` prefix map, built once by
/// [`Document::to_xml`] before serialization so every namespace the
/// document carried is declared exactly once on `<tt>` and each preserved
/// prefix keeps its original binding where possible (#1110/TT-W1).
///
/// The five core bindings (`xmlns=`, `xmlns:tt`, `xmlns:ttp`, `xmlns:tts`,
/// `xmlns:ttm`) are always emitted first — matching the pre-#1110 output
/// byte-for-byte. Well-known extension namespaces (tts: audio, IMSC,
/// EBU-TT, SMPTE-TT, XLink) are appended on demand whenever the document
/// uses them; preserved foreign namespaces are appended in document order
/// with their original prefixes. A foreign URI whose original prefix is
/// already taken by a different URI gets a generated `ttmfallbackN`
/// prefix: the URI round-trips unchanged, only the prefix spelling
/// differs (XML forbids one prefix bound to two URIs at the same time).
struct NamespaceMap {
    /// `(prefix, uri)` pairs in declaration order (core bindings first).
    bindings: Vec<(String, String)>,
    /// `xmlns:` declarations that were scoped below `<tt>` (foreign
    /// attributes or unknown elements carrying their own declarations).
    /// Re-emitted on `<tt>` so the document stays namespace-equivalent
    /// even though the original scope was narrower (TTML2 §7.2, #1110).
    scoped: Vec<(String, String)>,
    /// Scope-narrowing `xmlns:` overrides recorded in document order. XML
    /// NS 1.0 resolves a prefix on an element to the *last* matching
    /// declaration in document order, so when one prefix was overridden at
    /// several scopes the final URI per prefix is re-emitted last on `<tt>`,
    /// which makes every foreign attribute, element and text node re-parse
    /// with exactly the namespace it had originally (#1110/TT-W1).
    last_wins: Vec<(String, String)>,
}

/// The five core bindings always declared on `<tt>` (TTML2 §5.3).
const KEY_BINDINGS: &[(&str, &str)] = &[
    ("", NS_TT),
    ("tt", NS_TT),
    ("ttp", NS_TTP),
    ("tts", NS_TTS),
    ("ttm", NS_TTM),
];

/// Namespaces the crate models with a conventional prefix, declared on
/// `<tt>` on demand whenever the document uses them (TTML2 §5.3,
/// IMSC 1.1 §4, EBU-TT-D, SMPTE-TT, XLink §9.1.5).
const KNOWN_FOREIGN_NAMESPACES: &[(&str, &str)] = &[
    ("tta", NS_TTA),
    ("itts", NS_ITTS),
    ("ittp", NS_ITTP),
    ("ittm", NS_ITTM),
    ("ebuttm", NS_EBUTTM),
    ("ebutts", NS_EBUTTS),
    ("smpte", NS_SMPTE),
    ("xlink", NS_XLINK),
];

impl NamespaceMap {
    /// The map with only the five core bindings; extension bindings are
    /// added on demand during the collection pass or on first use.
    fn new() -> Self {
        NamespaceMap {
            bindings: KEY_BINDINGS
                .iter()
                .map(|(p, u)| (String::from(*p), String::from(*u)))
                .collect(),
            scoped: Vec::new(),
            last_wins: Vec::new(),
        }
    }

    /// The declarations to write after the five fixed core bindings.
    fn extra_bindings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.bindings[KEY_BINDINGS.len()..]
            .iter()
            .map(|(p, u)| (p.as_str(), u.as_str()))
    }

    /// Declare a crate-modeled extension namespace under its conventional
    /// prefix (no-op if already declared or the URI is core/unknown).
    fn add_well_known(&mut self, uri: &str) {
        if self.prefix_for(uri).is_some() {
            return;
        }
        if let Some((prefix, _)) = KNOWN_FOREIGN_NAMESPACES.iter().find(|(_, u)| *u == uri) {
            self.add_binding(prefix, uri);
        }
    }

    /// Record a preserved original binding. Core bindings may never be
    /// clobbered; a well-known or already-used prefix claiming a different
    /// URI falls through to [`NamespaceMap::add_fallback`].
    fn add_binding(&mut self, prefix: &str, uri: &str) {
        if prefix.is_empty() || KEY_BINDINGS.iter().any(|(p, _)| *p == prefix) {
            return;
        }
        if let Some((_, bound)) = self.bindings.iter().find(|(p, _)| p == prefix) {
            if bound != uri {
                if self.scoped.iter().any(|(p, u)| p == prefix && u == bound) {
                    // `uri` is the prefix's original binding and `bound` is
                    // an inner-scope override recorded in `scoped`: register
                    // the original under a fallback so the override can take
                    // the prefix verbatim on `<tt>` (#1110/TT-W1).
                    self.add_fallback(uri);
                }
                return;
            }
            return;
        }
        self.bindings
            .push((String::from(prefix), String::from(uri)));
    }

    /// Bind `uri` under its conventional prefix if free, else under a
    /// generated `ttmfallbackN` (never overwrites an existing binding).
    fn add_fallback(&mut self, uri: &str) {
        self.add_fallback_excluding(uri, None)
    }

    /// [`NamespaceMap::add_fallback`], ignoring a binding that is about to be
    /// taken over by another URI (the self-heal pass in
    /// [`collect_namespaces`] replaces a prefix's URI, so the previous owner
    /// must still get a declaration of its own even though it currently owns
    /// `exclude_prefix`).
    fn add_fallback_excluding(&mut self, uri: &str, exclude_prefix: Option<&str>) {
        let covered = self
            .bindings
            .iter()
            .any(|(p, u)| u == uri && exclude_prefix.is_none_or(|excluded| p != excluded));
        if covered {
            return;
        }
        if let Some((prefix, _)) = KNOWN_FOREIGN_NAMESPACES.iter().find(|(_, u)| *u == uri)
            && !self.bindings.iter().any(|(p, _)| p == prefix)
        {
            self.bindings
                .push((String::from(*prefix), String::from(uri)));
            return;
        }
        for n in 0u32.. {
            let prefix = format!("{FALLBACK_PREFIX}{n}");
            if !self.bindings.iter().any(|(p, _)| p == &prefix) {
                self.bindings.push((prefix, String::from(uri)));
                return;
            }
        }
    }

    /// Record an `xmlns:` declaration that scoped below `<tt>`. A
    /// declaration equal to an existing binding is a no-op; a *new* URI for
    /// an already-owned prefix is an inner-scope override, recorded in
    /// `scoped` (re-emitted on `<tt>`, which only widens the scope) and in
    /// document order in `last_wins` for the self-heal pass in
    /// [`collect_namespaces`] (#1110/TT-W1).
    fn add_scoped(&mut self, prefix: &str, uri: &str) {
        if prefix == "xml" || prefix == "xmlns" || uri == NS_XML || uri == NS_XMLNS {
            return;
        }
        // Already a `<tt>` declaration of this exact pair (core binding,
        // collected foreign binding, or earlier capture): nothing to record.
        if self.bindings.iter().any(|(p, u)| p == prefix && u == uri) {
            return;
        }
        if self.scoped.iter().any(|(p, u)| p == prefix && u == uri) {
            return;
        }
        // An earlier recorded override of this prefix was an outer-scope
        // binding superseded by this one in document order; drop it (the
        // surviving record is the last one, see `last_wins`).
        if let Some(prev) = self.scoped.iter().position(|(p, _)| p == prefix) {
            self.scoped.remove(prev);
        }
        // If the prefix is owned by a *different* URI (an ancestor
        // declaration, or a fallback that outer-scope content still needs),
        // record the override; it is re-emitted on `<tt>` after all other
        // declarations of the prefix. If the prefix is free, it simply
        // becomes a normal declaration.
        if self.bindings.iter().any(|(p, _)| p == prefix) {
            self.scoped.push((String::from(prefix), String::from(uri)));
        } else {
            self.bindings
                .push((String::from(prefix), String::from(uri)));
        }
        if let Some(prev) = self.last_wins.iter().position(|(p, _)| p == prefix) {
            self.last_wins.remove(prev);
        }
        self.last_wins
            .push((String::from(prefix), String::from(uri)));
    }

    /// The prefix currently bound to `uri`, if any.
    fn prefix_for(&self, uri: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|(_, u)| u == uri)
            .map(|(p, _)| p.as_str())
    }

    /// The prefix to write for a preserved attribute or element: the
    /// recorded original binding when it matches, else the first binding
    /// for the URI, else a fallback bound on the spot (self-healing for
    /// struct-built trees; `to_xml`'s collection pass declares everything
    /// up front, so normally the first two arms always hit).
    fn prefix_for_attr(&mut self, attr: &ForeignAttribute) -> String {
        let uri = match attr.namespace.as_deref() {
            Some(uri) => uri,
            None => return String::new(),
        };
        // A scoped override of this prefix (recorded by `add_scoped`) wins
        // for the prefix spelling the attribute originally used, so an
        // inner-scope `xmlns:v` re-binding is reproduced verbatim.
        if let Some(prefix) = attr.prefix.as_deref()
            && self.scoped.iter().any(|(p, u)| p == prefix && u == uri)
        {
            return String::from(prefix);
        }
        if let Some(prefix) = attr.prefix.as_deref()
            && self.bindings.iter().any(|(p, u)| p == prefix && u == uri)
        {
            return String::from(prefix);
        }
        if let Some(prefix) = self.prefix_for(uri) {
            return String::from(prefix);
        }
        self.add_fallback(uri);
        String::from(self.prefix_for(uri).unwrap_or(FALLBACK_PREFIX))
    }
}

/// Write preserved foreign attributes at the end of an element's opening
/// tag (TTML2 §7.2, #1110/TT-W1). Never skips one: an attribute whose
/// namespace is undeclared (only possible for trees built through the
/// struct API without going through `to_xml`'s collection pass) still
/// gets written under a fallback prefix.
fn serialize_ns_attrs(buf: &mut String, attrs: &[ForeignAttribute], ns: &mut NamespaceMap) {
    for attr in attrs {
        match attr.namespace.as_deref() {
            // Unprefixed attribute in no namespace (struct-built trees only;
            // a parsed document's unprefixed attributes are in the element's
            // default namespace and are modeled fields, never foreign).
            None => {
                buf.push_str(&format!(
                    r#" {}="{}""#,
                    xml_escape(&attr.local_name),
                    xml_escape(&attr.value)
                ));
            }
            Some(_uri) => {
                let prefix = ns.prefix_for_attr(attr);
                buf.push_str(&format!(
                    r#" {}:{}="{}""#,
                    prefix,
                    xml_escape(&attr.local_name),
                    xml_escape(&attr.value)
                ));
            }
        }
    }
}

/// Write `xml:space` (§7.7): `Some(Default)` comes from an explicit
/// `xml:space="default"`, `None` from its absence, so both spellings
/// round-trip distinctly (#1110/TT-W1).
fn serialize_xml_space(buf: &mut String, space: &Option<XmlSpace>) {
    if let Some(space) = space {
        buf.push_str(&format!(r#" xml:space="{}""#, space.name()));
    }
}

/// Write `xml:base` (TTML2 §7.4, #1110/TT-W1).
fn serialize_xml_base(buf: &mut String, base: &Option<String>) {
    serialize_opt_attr(buf, "xml:base", base);
}

/// Walk the whole document once in document order and build the namespace
/// map: every preserved original prefix binding is recorded (preserving the
/// spelling used in the source), and every namespace the document merely
/// *uses* (a modeled extension attribute, an IMSC/EBU/SMPTE/XLink value) is
/// declared under its conventional prefix. A preserved prefix that collides
/// with a different URI gets a `ttmfallbackN` binding so nothing is lost.
/// Iterative: span/metadata chains built via the struct API are unbounded.
fn collect_namespaces(tt: &mut TtElement) -> NamespaceMap {
    let mut ns = NamespaceMap::new();

    // Element-level extension attributes modeled by the crate.
    for attrs in [
        &tt.ittp_active_area,
        &tt.ittp_aspect_ratio,
        &tt.ittp_progressively_decodable,
    ] {
        if attrs.is_some() {
            ns.add_well_known(NS_ITTP);
        }
    }
    if tt.tts_extent.is_some() {
        ns.add_well_known(NS_TTS);
    }

    // Preserved foreign attributes and unknown-subtree namespaces, walked
    // iteratively from the root.
    let mut stack: Vec<WalkNode> = alloc::vec![WalkNode::Tt(tt)];
    while let Some(node) = stack.pop() {
        match node {
            WalkNode::Tt(tt) => {
                register_scoped(&mut ns, tt.scoped_namespaces.as_deref().unwrap_or_default());
                push_attrs(&mut ns, &tt.foreign_attributes);
                for u in &tt.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                if let Some(ref head) = tt.head {
                    stack.push(WalkNode::Head(head));
                }
                if let Some(ref body) = tt.body {
                    stack.push(WalkNode::Body(body));
                }
            }
            WalkNode::Head(head) => {
                push_attrs(&mut ns, &head.foreign_attributes);
                for u in &head.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                if let Some(ref resources) = head.resources {
                    stack.push(WalkNode::Resources(resources));
                }
                if let Some(ref styling) = head.styling {
                    stack.push(WalkNode::Styling(styling));
                }
                if let Some(ref layout) = head.layout {
                    stack.push(WalkNode::Layout(layout));
                }
                for m in &head.metadata {
                    stack.push(WalkNode::Meta(m));
                }
            }
            WalkNode::Body(body) => {
                push_attrs(&mut ns, &body.foreign_attributes);
                style_attrs_namespaces(&body.style_attributes, &mut ns);
                for u in &body.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for d in &body.divs {
                    stack.push(WalkNode::Div(d));
                }
                for m in &body.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &body.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::Div(div) => {
                push_attrs(&mut ns, &div.foreign_attributes);
                style_attrs_namespaces(&div.style_attributes, &mut ns);
                if div.smpte_background_image.is_some() {
                    ns.add_well_known(NS_SMPTE);
                }
                for u in &div.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for p in &div.paragraphs {
                    stack.push(WalkNode::P(p));
                }
                for img in &div.images {
                    stack.push(WalkNode::Image(img));
                }
                for a in &div.audio {
                    stack.push(WalkNode::Audio(a));
                }
                for m in &div.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &div.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::P(p) => {
                push_attrs(&mut ns, &p.foreign_attributes);
                style_attrs_namespaces(&p.style_attributes, &mut ns);
                push_inlines(&mut ns, &p.content, &mut stack);
                for u in &p.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for m in &p.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &p.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::Span(span) => {
                push_attrs(&mut ns, &span.foreign_attributes);
                style_attrs_namespaces(&span.style_attributes, &mut ns);
                push_inlines(&mut ns, &span.content, &mut stack);
                for u in &span.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for m in &span.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &span.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::Image(img) => {
                push_attrs(&mut ns, &img.foreign_attributes);
                style_attrs_namespaces(&img.style_attributes, &mut ns);
                if img.tts_extent.is_some() {
                    ns.add_well_known(NS_TTS);
                }
                if img.xlink_href.is_some()
                    || img.xlink_role.is_some()
                    || img.xlink_arcrole.is_some()
                    || img.xlink_title.is_some()
                    || img.xlink_show.is_some()
                {
                    ns.add_well_known(NS_XLINK);
                }
                for u in &img.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for s in &img.sources {
                    stack.push(WalkNode::Source(s));
                }
                for m in &img.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &img.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::Audio(audio) => {
                push_attrs(&mut ns, &audio.foreign_attributes);
                style_attrs_namespaces(&audio.style_attributes, &mut ns);
                for u in &audio.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
                for s in &audio.sources {
                    stack.push(WalkNode::Source(s));
                }
                for m in &audio.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for a in &audio.animations {
                    let AnimationChild::Set(set) = a;
                    stack.push(WalkNode::Set(set));
                }
            }
            WalkNode::Meta(meta) => match meta {
                MetadataChild::Metadata(m) => {
                    push_attrs(&mut ns, &m.foreign_attributes);
                    register_scoped(&mut ns, m.scoped_namespaces.as_deref().unwrap_or_default());
                    for c in &m.children {
                        stack.push(WalkNode::Meta(c));
                    }
                    for u in &m.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::TtmTitle(t)
                | MetadataChild::TtmDesc(t)
                | MetadataChild::TtmCopyright(t) => {
                    push_attrs(&mut ns, &t.foreign_attributes);
                    register_scoped(&mut ns, t.scoped_namespaces.as_deref().unwrap_or_default());
                    for u in &t.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::TtmAgent(a) => {
                    push_attrs(&mut ns, &a.foreign_attributes);
                    register_scoped(&mut ns, a.scoped_namespaces.as_deref().unwrap_or_default());
                    for n in &a.names {
                        stack.push(WalkNode::Name(n));
                    }
                    for u in &a.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::TtmName(n) => {
                    stack.push(WalkNode::Name(n));
                }
                MetadataChild::TtmItem(item) => {
                    push_attrs(&mut ns, &item.foreign_attributes);
                    register_scoped(
                        &mut ns,
                        item.scoped_namespaces.as_deref().unwrap_or_default(),
                    );
                    for i in &item.items {
                        stack.push(WalkNode::Item(i));
                    }
                    for u in &item.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::EbuttmDocumentMetadata(eb) => {
                    push_attrs(&mut ns, &eb.foreign_attributes);
                    register_scoped(&mut ns, eb.scoped_namespaces.as_deref().unwrap_or_default());
                    ns.add_well_known(NS_EBUTTM);
                    for c in &eb.children {
                        stack.push(WalkNode::Meta(c));
                    }
                    for u in &eb.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::EbuttmConformsToStandard(cs) => {
                    push_attrs(&mut ns, &cs.foreign_attributes);
                    register_scoped(&mut ns, cs.scoped_namespaces.as_deref().unwrap_or_default());
                    ns.add_well_known(NS_EBUTTM);
                }
                MetadataChild::IttmAltText(alt) => {
                    push_attrs(&mut ns, &alt.foreign_attributes);
                    register_scoped(
                        &mut ns,
                        alt.scoped_namespaces.as_deref().unwrap_or_default(),
                    );
                    ns.add_well_known(NS_ITTM);
                    for u in &alt.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                MetadataChild::Unknown(u) => {
                    stack.push(WalkNode::Unknown(u));
                }
            },
            WalkNode::Name(n) => {
                push_attrs(&mut ns, &n.foreign_attributes);
                register_scoped(&mut ns, n.scoped_namespaces.as_deref().unwrap_or_default());
                for u in &n.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Item(item) => {
                push_attrs(&mut ns, &item.foreign_attributes);
                register_scoped(
                    &mut ns,
                    item.scoped_namespaces.as_deref().unwrap_or_default(),
                );
                for i in &item.items {
                    stack.push(WalkNode::Item(i));
                }
                for u in &item.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Set(set) => {
                push_attrs(&mut ns, &set.foreign_attributes);
                style_attrs_namespaces(&set.style_attributes, &mut ns);
                for m in &set.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for u in &set.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Styling(styling) => {
                push_attrs(&mut ns, &styling.foreign_attributes);
                for i in &styling.initials {
                    push_attrs(&mut ns, &i.foreign_attributes);
                    style_attrs_namespaces(&i.style_attributes, &mut ns);
                    for u in &i.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                for s in &styling.styles {
                    push_attrs(&mut ns, &s.foreign_attributes);
                    style_attrs_namespaces(&s.style_attributes, &mut ns);
                    for u in &s.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                for u in &styling.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Layout(layout) => {
                push_attrs(&mut ns, &layout.foreign_attributes);
                for r in &layout.regions {
                    push_attrs(&mut ns, &r.foreign_attributes);
                    style_attrs_namespaces(&r.style_attributes, &mut ns);
                    for m in &r.metadata {
                        stack.push(WalkNode::Meta(m));
                    }
                    for a in &r.animations {
                        let AnimationChild::Set(set) = a;
                        stack.push(WalkNode::Set(set));
                    }
                    for st in &r.styles {
                        push_attrs(&mut ns, &st.foreign_attributes);
                        style_attrs_namespaces(&st.style_attributes, &mut ns);
                        for u in &st.unknown_children {
                            stack.push(WalkNode::Unknown(u));
                        }
                    }
                    for u in &r.unknown_children {
                        stack.push(WalkNode::Unknown(u));
                    }
                }
                for u in &layout.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Resources(resources) => {
                push_attrs(&mut ns, &resources.foreign_attributes);
                for d in &resources.data {
                    stack.push(WalkNode::Data(d));
                }
                for i in &resources.images {
                    stack.push(WalkNode::Image(i));
                }
                for a in &resources.audio {
                    stack.push(WalkNode::Audio(a));
                }
                for f in &resources.fonts {
                    stack.push(WalkNode::Font(f));
                }
                for m in &resources.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for u in &resources.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Data(data) => {
                push_attrs(&mut ns, &data.foreign_attributes);
                for c in &data.chunks {
                    push_attrs(&mut ns, &c.foreign_attributes);
                }
                for s in &data.sources {
                    stack.push(WalkNode::Source(s));
                }
                for m in &data.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for u in &data.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Font(font) => {
                push_attrs(&mut ns, &font.foreign_attributes);
                for s in &font.sources {
                    stack.push(WalkNode::Source(s));
                }
                for m in &font.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for u in &font.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Source(source) => {
                push_attrs(&mut ns, &source.foreign_attributes);
                if let Some(ref d) = source.data {
                    stack.push(WalkNode::Data(d));
                }
                for m in &source.metadata {
                    stack.push(WalkNode::Meta(m));
                }
                for u in &source.unknown_children {
                    stack.push(WalkNode::Unknown(u));
                }
            }
            WalkNode::Unknown(u) => {
                // The subtree's own scope declarations first: the local
                // scope must win over bindings inherited from ancestors
                // (#1110/TT-W1).
                register_scoped(&mut ns, u.scoped_namespaces.as_deref().unwrap_or_default());
                // Record the subtree's own binding plus its whole subtree.
                if let (Some(prefix), Some(uri)) = (u.prefix.as_deref(), u.namespace.as_deref()) {
                    ns.add_binding(prefix, uri);
                }
                push_attrs(&mut ns, &u.attributes);
                for c in &u.children {
                    if let UnknownNode::Element(e) = c {
                        stack.push(WalkNode::Unknown(e));
                    }
                }
            }
        }
    }

    // Final self-heal pass (XML NS 1.0 "last one wins"): for each prefix an
    // element re-declared mid-document, the *document-order-last* override
    // wins for every foreign node after it, including nodes whose captured
    // prefix was computed under an outer binding. Re-associate such nodes
    // (attributes, unknown elements and their subtrees) with the winning URI
    // and make sure that URI owns the prefix on `<tt>` — moving the previous
    // owner to a `ttmfallbackN` binding — so the whole document re-parses
    // with exactly the namespaces it resolved to originally (#1110/TT-W1).
    let overrides: Vec<(String, String)> = core::mem::take(&mut ns.last_wins);
    for (prefix, uri) in &overrides {
        let owner_index = ns.bindings.iter().position(|(p, _)| p == prefix);
        let wins_uri = match owner_index {
            Some(index) => {
                let previous = ns.bindings[index].1.clone();
                if previous == *uri {
                    uri.clone()
                } else {
                    // The override wins the prefix on `<tt>`. Hand whatever
                    // URI previously owned it here to a `ttmfallbackN`
                    // binding (outer-scope content still needs a binding for
                    // it), unless it already has a non-colliding one.
                    // `previous` still owns `prefix` at this point, so ask
                    // whether any *other* binding already covers it before
                    // handing it a fresh one.
                    ns.add_fallback_excluding(&previous, Some(prefix));
                    ns.bindings[index].1 = uri.clone();
                    uri.clone()
                }
            }
            None => continue,
        };
        // Drop any `scoped` re-emission for this prefix: the winning URI now
        // owns the single `<tt>` declaration (double declarations of one
        // prefix are not well-formed XML).
        ns.scoped.retain(|(p, _)| p != prefix);
        heal_last_wins(tt, prefix, &wins_uri);
    }

    ns
}
/// After a mid-document `xmlns:` override wins at `<tt>` scope (see the
/// self-heal pass in [`collect_namespaces`]), re-associate every preserved
/// item that *resolved* to the overridden URI through this prefix: an item
/// whose recorded prefix is `prefix` but whose captured URI differs from
/// the winner sat in the scope of the override, so re-pointing it keeps the
/// re-parsed namespaces exactly as they resolved before. Iterative: span
/// and unknown chains built via the struct API are unbounded (#1110/TT-W1).
fn heal_last_wins(tt: &mut TtElement, prefix: &str, uri: &str) {
    enum Mut<'a> {
        Tt(&'a mut TtElement),
        Head(&'a mut HeadElement),
        Body(&'a mut BodyElement),
        Div(&'a mut DivElement),
        P(&'a mut PElement),
        Span(&'a mut SpanElement),
        Image(&'a mut ImageElement),
        Audio(&'a mut AudioElement),
        Source(&'a mut SourceElement),
        Data(&'a mut DataElement),
        Style(&'a mut StyleElement),
        Meta(&'a mut MetadataChild),
        Unknown(&'a mut UnknownElement),
        Inline(&'a mut InlineContent),
    }

    fn heal_attrs(attrs: &mut [ForeignAttribute], prefix: &str, uri: &str) {
        for a in attrs {
            if a.prefix.as_deref() == Some(prefix) && a.namespace.as_deref() != Some(uri) {
                a.namespace = Some(String::from(uri));
            }
        }
    }

    let mut stack: Vec<Mut> = alloc::vec![Mut::Tt(tt)];
    while let Some(node) = stack.pop() {
        match node {
            Mut::Tt(tt) => {
                heal_attrs(&mut tt.foreign_attributes, prefix, uri);
                for u in &mut tt.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                if let Some(head) = &mut tt.head {
                    stack.push(Mut::Head(head));
                }
                if let Some(body) = &mut tt.body {
                    stack.push(Mut::Body(body));
                }
            }
            Mut::Head(head) => {
                heal_attrs(&mut head.foreign_attributes, prefix, uri);
                for u in &mut head.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                if let Some(styling) = &mut head.styling {
                    for st in &mut styling.styles {
                        stack.push(Mut::Style(st));
                    }
                }
                for m in &mut head.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Style(style) => {
                heal_attrs(&mut style.foreign_attributes, prefix, uri);
                for u in &mut style.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
            }
            Mut::Body(body) => {
                heal_attrs(&mut body.foreign_attributes, prefix, uri);
                for u in &mut body.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for d in &mut body.divs {
                    stack.push(Mut::Div(d));
                }
                for m in &mut body.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Div(div) => {
                heal_attrs(&mut div.foreign_attributes, prefix, uri);
                for u in &mut div.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for p in &mut div.paragraphs {
                    stack.push(Mut::P(p));
                }
                for img in &mut div.images {
                    stack.push(Mut::Image(img));
                }
                for a in &mut div.audio {
                    stack.push(Mut::Audio(a));
                }
                for m in &mut div.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::P(p) => {
                heal_attrs(&mut p.foreign_attributes, prefix, uri);
                for u in &mut p.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for c in &mut p.content {
                    stack.push(Mut::Inline(c));
                }
                for m in &mut p.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Span(span) => {
                heal_attrs(&mut span.foreign_attributes, prefix, uri);
                for u in &mut span.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for c in &mut span.content {
                    stack.push(Mut::Inline(c));
                }
                for m in &mut span.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Image(img) => {
                heal_attrs(&mut img.foreign_attributes, prefix, uri);
                for u in &mut img.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for sc in &mut img.sources {
                    stack.push(Mut::Source(sc));
                }
                for m in &mut img.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Audio(audio) => {
                heal_attrs(&mut audio.foreign_attributes, prefix, uri);
                for u in &mut audio.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                for sc in &mut audio.sources {
                    stack.push(Mut::Source(sc));
                }
                for m in &mut audio.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Source(source) => {
                heal_attrs(&mut source.foreign_attributes, prefix, uri);
                for u in &mut source.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
                if let Some(data) = &mut source.data {
                    stack.push(Mut::Data(data));
                }
                for m in &mut source.metadata {
                    stack.push(Mut::Meta(m));
                }
            }
            Mut::Data(data) => {
                heal_attrs(&mut data.foreign_attributes, prefix, uri);
                for c in &mut data.chunks {
                    heal_attrs(&mut c.foreign_attributes, prefix, uri);
                }
                for sc in &mut data.sources {
                    stack.push(Mut::Source(sc));
                }
                for m in &mut data.metadata {
                    stack.push(Mut::Meta(m));
                }
                for u in &mut data.unknown_children {
                    stack.push(Mut::Unknown(u));
                }
            }
            Mut::Inline(item) => match item {
                InlineContent::Span(span) => stack.push(Mut::Span(span)),
                InlineContent::Image(img) => stack.push(Mut::Image(img)),
                InlineContent::Audio(audio) => stack.push(Mut::Audio(audio)),
                _ => {}
            },
            Mut::Meta(meta) => match meta {
                MetadataChild::Metadata(m) => {
                    heal_attrs(&mut m.foreign_attributes, prefix, uri);
                    for u in &mut m.unknown_children {
                        stack.push(Mut::Unknown(u));
                    }
                    for c in &mut m.children {
                        stack.push(Mut::Meta(c));
                    }
                }
                MetadataChild::Unknown(u) => stack.push(Mut::Unknown(u)),
                _ => {}
            },
            Mut::Unknown(u) => {
                if u.prefix.as_deref() == Some(prefix) && u.namespace.as_deref() != Some(uri) {
                    u.namespace = Some(String::from(uri));
                }
                heal_attrs(&mut u.attributes, prefix, uri);
                for c in &mut u.children {
                    if let UnknownNode::Element(child) = c {
                        stack.push(Mut::Unknown(child));
                    }
                }
            }
        }
    }
}

// ─── Namespace collection helpers (#1110/TT-W1) ───────────────────

/// One node kind in the iterative namespace-collection walk. An explicit
/// enum (rather than `dyn`) keeps the walk allocation-light and lets every
/// element's own fields be visited without recursion.
enum WalkNode<'a> {
    Tt(&'a TtElement),
    Head(&'a HeadElement),
    Body(&'a BodyElement),
    Div(&'a DivElement),
    P(&'a PElement),
    Span(&'a SpanElement),
    Image(&'a ImageElement),
    Audio(&'a AudioElement),
    Meta(&'a MetadataChild),
    Name(&'a TtmNameElement),
    Item(&'a TtmItemElement),
    Set(&'a SetElement),
    Styling(&'a StylingElement),
    Layout(&'a LayoutElement),
    Resources(&'a ResourcesElement),
    Data(&'a DataElement),
    Font(&'a FontElement),
    Source(&'a SourceElement),
    Unknown(&'a UnknownElement),
}

/// Register every preserved attribute's `(prefix, uri)` binding.
fn push_attrs(ns: &mut NamespaceMap, attrs: &[ForeignAttribute]) {
    for attr in attrs {
        if let (Some(prefix), Some(uri)) = (attr.prefix.as_deref(), attr.namespace.as_deref()) {
            ns.add_binding(prefix, uri);
        }
        // Attributes carrying their own `xmlns:` declarations are recorded so
        // the prefixes they introduced survive to `<tt>`.
        register_scoped(ns, &attr.scoped_namespaces);
    }
}

/// Register the well-known namespaces a [`StyleAttributes`] value references
/// (a style attribute in namespace X requires `xmlns:` for X on `<tt>`).
fn style_attrs_namespaces(attrs: &StyleAttributes, ns: &mut NamespaceMap) {
    if style_attrs_use_ns(attrs, NS_TTS) {
        ns.add_well_known(NS_TTS);
    }
    if style_attrs_use_ns(attrs, NS_TTA) {
        ns.add_well_known(NS_TTA);
    }
    if style_attrs_use_ns(attrs, NS_ITTS) {
        ns.add_well_known(NS_ITTS);
    }
    if style_attrs_use_ns(attrs, NS_EBUTTS) {
        ns.add_well_known(NS_EBUTTS);
    }
}

/// Does any field of `attrs` belong to namespace `ns`? The per-URI check the
/// pre-#1110 `*_ns_needed` helpers each inlined, kept as one function.
fn style_attrs_use_ns(attrs: &StyleAttributes, ns: &str) -> bool {
    let fields: [(&str, &Option<String>); 59] = [
        (NS_TTS, &attrs.tts_background_color),
        (NS_TTS, &attrs.tts_background_clip),
        (NS_TTS, &attrs.tts_background_extent),
        (NS_TTS, &attrs.tts_background_image),
        (NS_TTS, &attrs.tts_background_origin),
        (NS_TTS, &attrs.tts_background_position),
        (NS_TTS, &attrs.tts_background_repeat),
        (NS_TTS, &attrs.tts_border),
        (NS_TTS, &attrs.tts_bpd),
        (NS_TTS, &attrs.tts_color),
        (NS_TTS, &attrs.tts_direction),
        (NS_TTS, &attrs.tts_disparity),
        (NS_TTS, &attrs.tts_display),
        (NS_TTS, &attrs.tts_display_align),
        (NS_TTS, &attrs.tts_extent),
        (NS_TTS, &attrs.tts_font_family),
        (NS_TTS, &attrs.tts_font_kerning),
        (NS_TTS, &attrs.tts_font_selection_strategy),
        (NS_TTS, &attrs.tts_font_shear),
        (NS_TTS, &attrs.tts_font_size),
        (NS_TTS, &attrs.tts_font_style),
        (NS_TTS, &attrs.tts_font_variant),
        (NS_TTS, &attrs.tts_font_weight),
        (NS_TTS, &attrs.tts_ipd),
        (NS_TTS, &attrs.tts_letter_spacing),
        (NS_TTS, &attrs.tts_line_height),
        (NS_TTS, &attrs.tts_line_shear),
        (NS_TTS, &attrs.tts_luminance_gain),
        (NS_TTS, &attrs.tts_opacity),
        (NS_TTS, &attrs.tts_origin),
        (NS_TTS, &attrs.tts_overflow),
        (NS_TTS, &attrs.tts_padding),
        (NS_TTS, &attrs.tts_position),
        (NS_TTS, &attrs.tts_ruby),
        (NS_TTS, &attrs.tts_ruby_align),
        (NS_TTS, &attrs.tts_ruby_position),
        (NS_TTS, &attrs.tts_ruby_reserve),
        (NS_TTS, &attrs.tts_shear),
        (NS_TTS, &attrs.tts_show_background),
        (NS_TTS, &attrs.tts_text_align),
        (NS_TTS, &attrs.tts_text_combine),
        (NS_TTS, &attrs.tts_text_decoration),
        (NS_TTS, &attrs.tts_text_emphasis),
        (NS_TTS, &attrs.tts_text_orientation),
        (NS_TTS, &attrs.tts_text_outline),
        (NS_TTS, &attrs.tts_text_shadow),
        (NS_TTS, &attrs.tts_unicode_bidi),
        (NS_TTS, &attrs.tts_visibility),
        (NS_TTS, &attrs.tts_wrap_option),
        (NS_TTS, &attrs.tts_writing_mode),
        (NS_TTS, &attrs.tts_z_index),
        (NS_TTA, &attrs.tta_gain),
        (NS_TTA, &attrs.tta_pan),
        (NS_TTA, &attrs.tta_pitch),
        (NS_TTA, &attrs.tta_speak),
        (NS_ITTS, &attrs.itts_forced_display),
        (NS_ITTS, &attrs.itts_fill_line_gap),
        (NS_EBUTTS, &attrs.ebutts_line_padding),
        (NS_EBUTTS, &attrs.ebutts_multi_row_align),
    ];
    fields
        .iter()
        .any(|(uri, value)| *uri == ns && value.is_some())
}

/// Record `xmlns:` declarations scoped to one element. A declaration equal to
/// an existing binding is a no-op; a different URI for an owned prefix is an
/// inner-scope override.
fn register_scoped(ns: &mut NamespaceMap, scoped: &[(String, String)]) {
    for (prefix, uri) in scoped {
        ns.add_scoped(prefix, uri);
    }
}

/// Push the inline children of a `<p>`/`<span>` onto the walk stack.
fn push_inlines<'a>(
    ns: &mut NamespaceMap,
    content: &'a [InlineContent],
    stack: &mut Vec<WalkNode<'a>>,
) {
    for item in content {
        match item {
            InlineContent::Text(_) => {}
            InlineContent::Span(span) => {
                push_attrs(ns, &span.foreign_attributes);
                style_attrs_namespaces(&span.style_attributes, ns);
                stack.push(WalkNode::Span(span));
            }
            InlineContent::Image(img) => stack.push(WalkNode::Image(img)),
            InlineContent::Audio(audio) => stack.push(WalkNode::Audio(audio)),
            InlineContent::Br(br) => {
                push_attrs(ns, &br.foreign_attributes);
                style_attrs_namespaces(&br.style_attributes, ns);
            }
        }
    }
}

// ─── Element serialization ────────────────────────────────────────
//
// Every emitter takes the shared [`NamespaceMap`] so preserved foreign
// attributes and unknown subtrees can be re-emitted under the prefix
// bindings `collect_namespaces` gathered from the whole document
// (#1110/TT-W1).

/// Serialize the root `<tt>` element — TTML2 §8.1.1.
#[allow(clippy::too_many_lines)]
fn serialize_tt_element(tt: &TtElement, buf: &mut String, indent: usize, ns: &mut NamespaceMap) {
    let ind = "  ".repeat(indent);
    buf.push_str(&ind);
    buf.push_str("<tt");
    // The five core bindings, byte-identical to the pre-#1110 output.
    for (prefix, uri) in KEY_BINDINGS {
        if prefix.is_empty() {
            buf.push_str(&format!(r#" xmlns="{}""#, uri));
        } else {
            buf.push_str(&format!(r#" xmlns:{prefix}="{}""#, uri));
        }
    }
    // Extension and preserved bindings collected from the document.
    let extras: Vec<(String, String)> = ns
        .extra_bindings()
        .map(|(p, u)| (String::from(p), String::from(u)))
        .collect();
    for (prefix, uri) in &extras {
        buf.push_str(&format!(r#" xmlns:{prefix}="{}""#, xml_escape(uri)));
    }
    // Scope-narrowing overrides, declared last so they win in document
    // order (XML NS 1.0 "last one wins").
    let scoped: Vec<(String, String)> = ns.scoped.clone();
    for (prefix, uri) in &scoped {
        buf.push_str(&format!(r#" xmlns:{prefix}="{}""#, xml_escape(uri)));
    }

    serialize_opt_attr(buf, "xml:lang", &tt.xml_lang);
    serialize_opt_attr(buf, "xml:id", &tt.xml_id);
    serialize_xml_space(buf, &tt.xml_space);
    serialize_xml_base(buf, &tt.xml_base);

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
        "ttp:inferProcessorProfileMethod",
        &tt.ttp_infer_processor_profile_method,
    );
    serialize_opt_attr(
        buf,
        "ttp:inferProcessorProfileSource",
        &tt.ttp_infer_processor_profile_source,
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

    serialize_opt_attr(buf, "tts:extent", &tt.tts_extent);
    serialize_opt_attr(buf, "ittp:activeArea", &tt.ittp_active_area);
    serialize_opt_attr(buf, "ittp:aspectRatio", &tt.ittp_aspect_ratio);
    serialize_opt_attr(
        buf,
        "ittp:progressivelyDecodable",
        &tt.ittp_progressively_decodable,
    );

    serialize_ns_attrs(buf, &tt.foreign_attributes, ns);
    buf.push_str(">\n");

    if let Some(ref head) = tt.head {
        serialize_head_element(head, buf, indent + 1, ns);
    }
    if let Some(ref body) = tt.body {
        serialize_body_element(body, buf, indent + 1, ns);
    }
    for unknown in &tt.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }

    buf.push_str(&format!("{}</tt>\n", ind));
}

/// Serialize a preserved foreign subtree (§7.2/§7.3, #1110/TT-W1).
fn serialize_unknown_element(
    elem: &UnknownElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    let name = match elem.prefix.as_deref() {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", elem.local_name),
        _ => elem.local_name.clone(),
    };
    // The element's own scope declarations are mirrored into the map so any
    // descendant using one of its prefixes resolves (#1110/TT-W1).
    let scoped = elem.scoped_namespaces.clone().unwrap_or_default();
    register_scoped(ns, &scoped);
    if let (Some(prefix), Some(uri)) = (elem.prefix.as_deref(), elem.namespace.as_deref())
        && !prefix.is_empty()
    {
        ns.add_binding(prefix, uri);
    }

    buf.push_str(&format!("{ind}<{name}"));
    serialize_ns_attrs(buf, &elem.attributes, ns);
    if elem.children.is_empty() {
        buf.push_str("/>\n");
        return;
    }
    let has_element_child = elem
        .children
        .iter()
        .any(|c| matches!(c, UnknownNode::Element(_)));
    if !has_element_child {
        // Text-only subtree: keep it on one line so no spurious whitespace
        // text node is introduced.
        buf.push('>');
        for child in &elem.children {
            if let UnknownNode::Text(t) = child {
                buf.push_str(&xml_escape(t));
            }
        }
        buf.push_str(&format!("</{name}>\n"));
        return;
    }
    buf.push_str(">\n");
    for child in &elem.children {
        match child {
            UnknownNode::Element(e) => serialize_unknown_element(e, buf, indent + 1, ns),
            UnknownNode::Text(t) => {
                let text = t.trim();
                if text.is_empty() {
                    continue;
                }
                buf.push_str(&format!(
                    "{}{}\n",
                    "  ".repeat(indent + 1),
                    xml_escape(text)
                ));
            }
        }
    }
    buf.push_str(&format!("{ind}</{name}>\n"));
}

/// Serialize `<head>` — TTML2 §8.1.2.
fn serialize_head_element(
    head: &HeadElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<head"));
    serialize_opt_attr(buf, "xml:id", &head.xml_id);
    serialize_opt_attr(buf, "xml:lang", &head.xml_lang);
    serialize_xml_space(buf, &head.xml_space);
    serialize_xml_base(buf, &head.xml_base);
    serialize_ns_attrs(buf, &head.foreign_attributes, ns);
    buf.push_str(">\n");

    for meta in &head.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    if let Some(ref styling) = head.styling {
        serialize_styling_element(styling, buf, indent + 1, ns);
    }
    if let Some(ref layout) = head.layout {
        serialize_layout_element(layout, buf, indent + 1, ns);
    }
    if let Some(ref resources) = head.resources {
        serialize_resources_element(resources, buf, indent + 1, ns);
    }
    for unknown in &head.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }

    buf.push_str(&format!("{ind}</head>\n"));
}

/// Serialize `<body>` — TTML2 §8.1.3.
fn serialize_body_element(
    body: &BodyElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<body"));
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
    serialize_xml_space(buf, &body.xml_space);
    serialize_xml_base(buf, &body.xml_base);
    serialize_ns_attrs(buf, &body.foreign_attributes, ns);

    let has_children = !body.divs.is_empty()
        || !body.metadata.is_empty()
        || !body.animations.is_empty()
        || !body.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &body.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &body.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for div in &body.divs {
        serialize_div_element(div, buf, indent + 1, ns);
    }
    for unknown in &body.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</body>\n"));
}

/// Serialize `<div>` — TTML2 §8.1.4.
fn serialize_div_element(div: &DivElement, buf: &mut String, indent: usize, ns: &mut NamespaceMap) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<div"));
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
    serialize_xml_space(buf, &div.xml_space);
    serialize_xml_base(buf, &div.xml_base);
    serialize_ns_attrs(buf, &div.foreign_attributes, ns);

    let has_children = !div.paragraphs.is_empty()
        || !div.images.is_empty()
        || !div.audio.is_empty()
        || !div.metadata.is_empty()
        || !div.animations.is_empty()
        || !div.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &div.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &div.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for p in &div.paragraphs {
        serialize_p_element(p, buf, indent + 1, ns);
    }
    for img in &div.images {
        serialize_image_element(img, buf, indent + 1, ns);
    }
    for audio in &div.audio {
        serialize_audio_element(audio, buf, indent + 1, ns);
    }
    for unknown in &div.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</div>\n"));
}

/// Serialize `<p>` — TTML2 §8.1.5.
///
/// Content children are written by an iterative walk (see
/// [`serialize_inline_content`]) so a `<span>` chain built through the struct
/// API cannot overflow the stack; metadata/animation children are hoisted
/// before the inline content, glued to the opening tag so no whitespace text
/// node appears where the source had none.
fn serialize_p_element(p: &PElement, buf: &mut String, indent: usize, ns: &mut NamespaceMap) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<p"));
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
    serialize_xml_space(buf, &p.xml_space);
    serialize_xml_base(buf, &p.xml_base);
    serialize_ns_attrs(buf, &p.foreign_attributes, ns);

    let has_children = !p.content.is_empty()
        || !p.metadata.is_empty()
        || !p.animations.is_empty()
        || !p.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push('>');
    for meta in &p.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &p.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for item in &p.content {
        serialize_inline_content(item, buf, ns);
    }
    for unknown in &p.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str("</p>\n");
}

/// Serialize inline content (text, `<span>`, `<br>`, `<image>`, `<audio>`).
///
/// Iterative (explicit stack), not recursive: a `<span>` chain built via the
/// struct API can nest far deeper than `MAX_NESTING_DEPTH` (which only bounds
/// `Document::parse_str`), and a recursive walk would overflow the stack.
fn serialize_inline_content(content: &InlineContent, buf: &mut String, ns: &mut NamespaceMap) {
    enum Frame<'a> {
        Node(&'a InlineContent),
        Close(&'static str),
    }

    let mut stack: Vec<Frame> = alloc::vec![Frame::Node(content)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Node(InlineContent::Text(text)) => {
                buf.push_str(&xml_escape(text));
            }
            Frame::Node(InlineContent::Span(span)) => {
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
                serialize_opt_attr(buf, "condition", &span.condition);
                serialize_style_attrs(&span.style_attributes, buf);
                serialize_opt_attr(buf, "xml:id", &span.xml_id);
                serialize_opt_attr(buf, "xml:lang", &span.xml_lang);
                serialize_xml_space(buf, &span.xml_space);
                serialize_xml_base(buf, &span.xml_base);
                serialize_ns_attrs(buf, &span.foreign_attributes, ns);

                let has_children = !span.content.is_empty()
                    || !span.metadata.is_empty()
                    || !span.animations.is_empty()
                    || !span.unknown_children.is_empty();
                if !has_children {
                    buf.push_str("/>");
                } else {
                    buf.push('>');
                    stack.push(Frame::Close("span"));
                    for item in span.content.iter().rev() {
                        stack.push(Frame::Node(item));
                    }
                }
            }
            Frame::Node(InlineContent::Br(br)) => {
                buf.push_str("<br");
                serialize_opt_attr(buf, "style", &br.style);
                serialize_opt_attr(buf, "condition", &br.condition);
                serialize_opt_attr(buf, "ttm:role", &br.ttm_role);
                serialize_opt_attr(buf, "ttm:roleSource", &br.ttm_role_source);
                serialize_opt_attr(buf, "xml:id", &br.xml_id);
                serialize_opt_attr(buf, "xml:lang", &br.xml_lang);
                serialize_xml_space(buf, &br.xml_space);
                serialize_xml_base(buf, &br.xml_base);
                serialize_style_attrs(&br.style_attributes, buf);
                serialize_ns_attrs(buf, &br.foreign_attributes, ns);
                buf.push_str("/>");
            }
            Frame::Node(InlineContent::Image(img)) => {
                serialize_inline_image(img, buf, ns);
            }
            Frame::Node(InlineContent::Audio(audio)) => {
                serialize_inline_audio(audio, buf, ns);
            }
            Frame::Close(tag) => {
                buf.push_str("</");
                buf.push_str(tag);
                buf.push('>');
            }
        }
    }
}

/// Inline `<image>`: same attributes as the block form, written on one line
/// (it is inline content of `<p>`/`<span>` — TTML2 §9.1.5).
fn serialize_inline_image(image: &ImageElement, buf: &mut String, ns: &mut NamespaceMap) {
    buf.push_str("<image");
    serialize_image_attrs(image, buf, ns);
    buf.push_str("/>");
}

/// Inline `<audio>`: attributes plus optional character data (§9.1.1).
fn serialize_inline_audio(audio: &AudioElement, buf: &mut String, ns: &mut NamespaceMap) {
    buf.push_str("<audio");
    serialize_audio_attrs(audio, buf, ns);
    match audio.text.as_deref() {
        Some(text) if !text.is_empty() => {
            buf.push('>');
            buf.push_str(&xml_escape(text));
            buf.push_str("</audio>");
        }
        _ => buf.push_str("/>"),
    }
}

/// The `<image>` attribute set, shared by the block and inline forms.
fn serialize_image_attrs(image: &ImageElement, buf: &mut String, ns: &mut NamespaceMap) {
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
    serialize_opt_attr(buf, "ttm:role", &image.ttm_role);
    serialize_opt_attr(buf, "ttm:roleSource", &image.ttm_role_source);
    serialize_opt_attr(buf, "src", &image.src);
    serialize_opt_attr(buf, "type", &image.type_);
    // `tts:extent` lives in its own field on ImageElement; skip the copy in
    // `style_attributes` so it is never written twice.
    serialize_opt_attr(buf, "tts:extent", &image.tts_extent);
    serialize_style_attrs_skip_extent(&image.style_attributes, buf);
    serialize_opt_attr(buf, "xlink:href", &image.xlink_href);
    serialize_opt_attr(buf, "xlink:role", &image.xlink_role);
    serialize_opt_attr(buf, "xlink:arcrole", &image.xlink_arcrole);
    serialize_opt_attr(buf, "xlink:title", &image.xlink_title);
    serialize_opt_attr(buf, "xlink:show", &image.xlink_show);
    serialize_opt_attr(buf, "xml:id", &image.xml_id);
    serialize_opt_attr(buf, "xml:lang", &image.xml_lang);
    serialize_xml_space(buf, &image.xml_space);
    serialize_xml_base(buf, &image.xml_base);
    serialize_ns_attrs(buf, &image.foreign_attributes, ns);
}

/// The `<audio>` attribute set, shared by the block and inline forms.
fn serialize_audio_attrs(audio: &AudioElement, buf: &mut String, ns: &mut NamespaceMap) {
    serialize_common_timing_attrs(
        buf,
        audio.begin.as_deref(),
        audio.dur.as_deref(),
        audio.end.as_deref(),
        audio.time_container.as_deref(),
    );
    serialize_opt_attr(buf, "clipBegin", &audio.clip_begin);
    serialize_opt_attr(buf, "clipEnd", &audio.clip_end);
    serialize_opt_attr(buf, "region", &audio.region);
    serialize_opt_attr(buf, "style", &audio.style);
    serialize_opt_attr(buf, "animate", &audio.animate);
    serialize_opt_attr(buf, "condition", &audio.condition);
    serialize_opt_attr(buf, "ttm:role", &audio.ttm_role);
    serialize_opt_attr(buf, "ttm:roleSource", &audio.ttm_role_source);
    serialize_opt_attr(buf, "src", &audio.src);
    serialize_opt_attr(buf, "type", &audio.type_);
    serialize_style_attrs(&audio.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &audio.xml_id);
    serialize_opt_attr(buf, "xml:lang", &audio.xml_lang);
    serialize_xml_space(buf, &audio.xml_space);
    serialize_xml_base(buf, &audio.xml_base);
    serialize_ns_attrs(buf, &audio.foreign_attributes, ns);
}

/// Serialize an `<image>` child of `<div>`/`<resources>` (TTML2 §9.1.5).
fn serialize_image_element(
    image: &ImageElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<image"));
    serialize_image_attrs(image, buf, ns);

    let has_children = !image.metadata.is_empty()
        || !image.animations.is_empty()
        || !image.sources.is_empty()
        || !image.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &image.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &image.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for src in &image.sources {
        serialize_source_element(src, buf, indent + 1, ns);
    }
    for unknown in &image.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</image>\n"));
}

/// Serialize an `<audio>` child of `<div>`/`<resources>` (TTML2 §9.1.1).
fn serialize_audio_element(
    audio: &AudioElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<audio"));
    serialize_audio_attrs(audio, buf, ns);

    let has_element_children = !audio.metadata.is_empty()
        || !audio.animations.is_empty()
        || !audio.sources.is_empty()
        || !audio.unknown_children.is_empty();
    match (audio.text.as_deref(), has_element_children) {
        (None, false) => buf.push_str("/>\n"),
        (Some(""), false) => buf.push_str("/>\n"),
        (Some(text), false) => {
            buf.push('>');
            buf.push_str(&xml_escape(text));
            buf.push_str("</audio>\n");
        }
        (None, true) => {
            buf.push_str(">\n");
            for meta in &audio.metadata {
                serialize_metadata_child(meta, buf, indent + 1, ns);
            }
            for anim in &audio.animations {
                serialize_animation_child(anim, buf, indent + 1, ns);
            }
            for src in &audio.sources {
                serialize_source_element(src, buf, indent + 1, ns);
            }
            for unknown in &audio.unknown_children {
                serialize_unknown_element(unknown, buf, indent + 1, ns);
            }
            buf.push_str(&format!("{ind}</audio>\n"));
        }
        (Some(text), true) => {
            buf.push_str(">\n");
            if !text.trim().is_empty() {
                buf.push_str(&format!(
                    "{}{}\n",
                    "  ".repeat(indent + 1),
                    xml_escape(text.trim())
                ));
            }
            for meta in &audio.metadata {
                serialize_metadata_child(meta, buf, indent + 1, ns);
            }
            for anim in &audio.animations {
                serialize_animation_child(anim, buf, indent + 1, ns);
            }
            for src in &audio.sources {
                serialize_source_element(src, buf, indent + 1, ns);
            }
            for unknown in &audio.unknown_children {
                serialize_unknown_element(unknown, buf, indent + 1, ns);
            }
            buf.push_str(&format!("{ind}</audio>\n"));
        }
    }
}

/// Serialize `<resources>` — TTML2 §9.1.6.
fn serialize_resources_element(
    resources: &ResourcesElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<resources"));
    serialize_opt_attr(buf, "xml:id", &resources.xml_id);
    serialize_opt_attr(buf, "xml:lang", &resources.xml_lang);
    serialize_xml_space(buf, &resources.xml_space);
    serialize_xml_base(buf, &resources.xml_base);
    serialize_ns_attrs(buf, &resources.foreign_attributes, ns);
    buf.push_str(">\n");
    for meta in &resources.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for data in &resources.data {
        serialize_data_element(data, buf, indent + 1, ns);
    }
    for img in &resources.images {
        serialize_image_element(img, buf, indent + 1, ns);
    }
    for audio in &resources.audio {
        serialize_audio_element(audio, buf, indent + 1, ns);
    }
    for font in &resources.fonts {
        serialize_font_element(font, buf, indent + 1, ns);
    }
    for unknown in &resources.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</resources>\n"));
}

/// Serialize `<data>` — TTML2 §9.1.3 (inline text, `chunk`+, or `source`+).
fn serialize_data_element(
    data: &DataElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<data"));
    serialize_opt_attr(buf, "xml:id", &data.xml_id);
    serialize_opt_attr(buf, "xml:lang", &data.xml_lang);
    serialize_xml_space(buf, &data.xml_space);
    serialize_xml_base(buf, &data.xml_base);
    serialize_opt_attr(buf, "condition", &data.condition);
    serialize_opt_attr(buf, "encoding", &data.encoding);
    serialize_opt_attr(buf, "format", &data.format);
    serialize_opt_attr(buf, "length", &data.length);
    serialize_opt_attr(buf, "src", &data.src);
    serialize_opt_attr(buf, "type", &data.type_);
    serialize_opt_attr(buf, "ttm:role", &data.ttm_role);
    serialize_opt_attr(buf, "ttm:roleSource", &data.ttm_role_source);
    serialize_ns_attrs(buf, &data.foreign_attributes, ns);

    let has_element_children = !data.metadata.is_empty()
        || !data.chunks.is_empty()
        || !data.sources.is_empty()
        || !data.unknown_children.is_empty();
    let has_text = data.text.as_deref().is_some_and(|t| !t.is_empty());
    if !has_element_children && !has_text {
        buf.push_str("/>\n");
        return;
    }
    if !has_element_children {
        buf.push('>');
        buf.push_str(&xml_escape(data.text.as_deref().unwrap_or("")));
        buf.push_str("</data>\n");
        return;
    }
    buf.push_str(">\n");
    if let Some(text) = data.text.as_deref().filter(|t| !t.trim().is_empty()) {
        buf.push_str(&format!(
            "{}{}\n",
            "  ".repeat(indent + 1),
            xml_escape(text.trim())
        ));
    }
    for meta in &data.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for chunk in &data.chunks {
        serialize_chunk_element(chunk, buf, indent + 1, ns);
    }
    for src in &data.sources {
        serialize_source_element(src, buf, indent + 1, ns);
    }
    for unknown in &data.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</data>\n"));
}

/// Serialize `<chunk>` — TTML2 §9.1.2.
fn serialize_chunk_element(
    chunk: &ChunkElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<chunk"));
    serialize_opt_attr(buf, "xml:id", &chunk.xml_id);
    serialize_xml_base(buf, &chunk.xml_base);
    serialize_opt_attr(buf, "condition", &chunk.condition);
    serialize_opt_attr(buf, "encoding", &chunk.encoding);
    serialize_opt_attr(buf, "length", &chunk.length);
    serialize_ns_attrs(buf, &chunk.foreign_attributes, ns);
    match chunk.text.as_deref() {
        Some(text) if !text.is_empty() => {
            buf.push('>');
            buf.push_str(&xml_escape(text));
            buf.push_str("</chunk>\n");
        }
        _ => buf.push_str("/>\n"),
    }
}

/// Serialize `<font>` — TTML2 §9.1.4.
fn serialize_font_element(
    font: &FontElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<font"));
    serialize_opt_attr(buf, "xml:id", &font.xml_id);
    serialize_opt_attr(buf, "xml:lang", &font.xml_lang);
    serialize_xml_space(buf, &font.xml_space);
    serialize_xml_base(buf, &font.xml_base);
    serialize_opt_attr(buf, "condition", &font.condition);
    serialize_opt_attr(buf, "family", &font.family);
    serialize_opt_attr(buf, "range", &font.range);
    serialize_opt_attr(buf, "style", &font.style_);
    serialize_opt_attr(buf, "src", &font.src);
    serialize_opt_attr(buf, "type", &font.type_);
    serialize_opt_attr(buf, "weight", &font.weight);
    serialize_opt_attr(buf, "ttm:role", &font.ttm_role);
    serialize_opt_attr(buf, "ttm:roleSource", &font.ttm_role_source);
    serialize_ns_attrs(buf, &font.foreign_attributes, ns);

    let has_children = !font.metadata.is_empty()
        || !font.animations.is_empty()
        || !font.sources.is_empty()
        || !font.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &font.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &font.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for src in &font.sources {
        serialize_source_element(src, buf, indent + 1, ns);
    }
    for unknown in &font.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</font>\n"));
}

/// Serialize `<source>` — TTML2 §9.1.7.
fn serialize_source_element(
    source: &SourceElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<source"));
    serialize_opt_attr(buf, "xml:id", &source.xml_id);
    serialize_opt_attr(buf, "xml:lang", &source.xml_lang);
    serialize_xml_space(buf, &source.xml_space);
    serialize_xml_base(buf, &source.xml_base);
    serialize_opt_attr(buf, "condition", &source.condition);
    serialize_opt_attr(buf, "format", &source.format);
    serialize_opt_attr(buf, "src", &source.src);
    serialize_opt_attr(buf, "type", &source.type_);
    serialize_opt_attr(buf, "ttm:role", &source.ttm_role);
    serialize_opt_attr(buf, "ttm:roleSource", &source.ttm_role_source);
    serialize_ns_attrs(buf, &source.foreign_attributes, ns);

    let has_children =
        !source.metadata.is_empty() || source.data.is_some() || !source.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &source.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    if let Some(ref data) = source.data {
        serialize_data_element(data, buf, indent + 1, ns);
    }
    for unknown in &source.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</source>\n"));
}

/// Serialize a metadata child — TTML2 §14.1.
///
/// Iterative (explicit stack), not recursive: `<metadata>` and
/// `<ebuttm:documentMetadata>` can nest far deeper than `MAX_NESTING_DEPTH`
/// once built via the struct API.
fn serialize_metadata_child(
    child: &MetadataChild,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    enum Frame<'a> {
        Node(&'a MetadataChild, usize),
        Close(&'static str, usize),
    }

    let mut stack: Vec<Frame> = alloc::vec![Frame::Node(child, indent)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Node(MetadataChild::Metadata(m), depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}<metadata"));
                serialize_opt_attr(buf, "xml:id", &m.xml_id);
                serialize_opt_attr(buf, "xml:lang", &m.xml_lang);
                serialize_xml_space(buf, &m.xml_space);
                serialize_xml_base(buf, &m.xml_base);
                serialize_opt_attr(buf, "condition", &m.condition);
                serialize_ns_attrs(buf, &m.foreign_attributes, ns);
                let scoped = m.scoped_namespaces.clone().unwrap_or_default();
                register_scoped(ns, &scoped);
                let has_children = !m.children.is_empty() || !m.unknown_children.is_empty();
                if !has_children {
                    buf.push_str("/>\n");
                    continue;
                }
                buf.push_str(">\n");
                stack.push(Frame::Close("metadata", depth));
                for c in m.children.iter().rev() {
                    stack.push(Frame::Node(c, depth + 1));
                }
            }
            Frame::Node(MetadataChild::TtmTitle(t), depth) => {
                serialize_ttm_text_like("ttm:title", t, buf, depth, ns);
            }
            Frame::Node(MetadataChild::TtmDesc(t), depth) => {
                serialize_ttm_text_like("ttm:desc", t, buf, depth, ns);
            }
            Frame::Node(MetadataChild::TtmCopyright(t), depth) => {
                serialize_ttm_text_like("ttm:copyright", t, buf, depth, ns);
            }
            Frame::Node(MetadataChild::TtmAgent(a), depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}<ttm:agent"));
                serialize_opt_attr(buf, "type", &a.type_);
                serialize_opt_attr(buf, "xml:id", &a.xml_id);
                serialize_opt_attr(buf, "xml:lang", &a.xml_lang);
                serialize_xml_space(buf, &a.xml_space);
                serialize_xml_base(buf, &a.xml_base);
                serialize_opt_attr(buf, "condition", &a.condition);
                serialize_ns_attrs(buf, &a.foreign_attributes, ns);
                let scoped = a.scoped_namespaces.clone().unwrap_or_default();
                register_scoped(ns, &scoped);
                buf.push_str(">\n");
                for name in &a.names {
                    serialize_ttm_name(name, buf, depth + 1, ns);
                }
                for u in &a.unknown_children {
                    serialize_unknown_element(u, buf, depth + 1, ns);
                }
                buf.push_str(&format!("{ind}</ttm:agent>\n"));
            }
            Frame::Node(MetadataChild::TtmName(n), depth) => {
                serialize_ttm_name(n, buf, depth, ns);
            }
            Frame::Node(MetadataChild::TtmItem(item), depth) => {
                serialize_ttm_item(item, buf, depth, ns);
            }
            Frame::Node(MetadataChild::EbuttmDocumentMetadata(eb), depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}<ebuttm:documentMetadata"));
                serialize_ns_attrs(buf, &eb.foreign_attributes, ns);
                let scoped = eb.scoped_namespaces.clone().unwrap_or_default();
                register_scoped(ns, &scoped);
                buf.push_str(">\n");
                stack.push(Frame::Close("ebuttm:documentMetadata", depth));
                for c in eb.children.iter().rev() {
                    stack.push(Frame::Node(c, depth + 1));
                }
                for u in eb.unknown_children.iter().rev() {
                    serialize_unknown_element(u, buf, depth + 1, ns);
                }
            }
            Frame::Node(MetadataChild::EbuttmConformsToStandard(cs), depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}<ebuttm:conformsToStandard"));
                serialize_ns_attrs(buf, &cs.foreign_attributes, ns);
                let scoped = cs.scoped_namespaces.clone().unwrap_or_default();
                register_scoped(ns, &scoped);
                buf.push('>');
                buf.push_str(&xml_escape(&cs.text));
                buf.push_str("</ebuttm:conformsToStandard>\n");
            }
            Frame::Node(MetadataChild::IttmAltText(alt), depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}<ittm:altText"));
                serialize_opt_attr(buf, "xml:id", &alt.xml_id);
                serialize_opt_attr(buf, "xml:lang", &alt.xml_lang);
                serialize_xml_space(buf, &alt.xml_space);
                serialize_xml_base(buf, &alt.xml_base);
                serialize_ns_attrs(buf, &alt.foreign_attributes, ns);
                let scoped = alt.scoped_namespaces.clone().unwrap_or_default();
                register_scoped(ns, &scoped);
                buf.push('>');
                buf.push_str(&xml_escape(&alt.text));
                buf.push_str("</ittm:altText>\n");
            }
            Frame::Node(MetadataChild::Unknown(u), depth) => {
                serialize_unknown_element(u, buf, depth, ns);
            }
            Frame::Close(tag, depth) => {
                let ind = "  ".repeat(depth);
                buf.push_str(&format!("{ind}</{tag}>\n"));
            }
        }
    }
}

/// Serialize a text-only TT Metadata element (`ttm:title`/`ttm:desc`/
/// `ttm:copyright`) with its full attribute set — TTML2 §14.1.4/§14.1.5/§14.1.8.
fn serialize_ttm_text_like(
    tag: &str,
    t: &TtmTextElement,
    buf: &mut String,
    depth: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(depth);
    buf.push_str(&format!("{ind}<{tag}"));
    serialize_opt_attr(buf, "xml:id", &t.xml_id);
    serialize_opt_attr(buf, "xml:lang", &t.xml_lang);
    serialize_xml_space(buf, &t.xml_space);
    serialize_xml_base(buf, &t.xml_base);
    serialize_opt_attr(buf, "condition", &t.condition);
    serialize_ns_attrs(buf, &t.foreign_attributes, ns);
    let scoped = t.scoped_namespaces.clone().unwrap_or_default();
    register_scoped(ns, &scoped);
    buf.push('>');
    buf.push_str(&xml_escape(&t.text));
    buf.push_str(&format!("</{tag}>\n"));
}

/// Serialize `<ttm:name>` — TTML2 §14.1.7.
fn serialize_ttm_name(n: &TtmNameElement, buf: &mut String, depth: usize, ns: &mut NamespaceMap) {
    let ind = "  ".repeat(depth);
    buf.push_str(&format!("{ind}<ttm:name"));
    serialize_opt_attr(buf, "type", &n.type_);
    serialize_opt_attr(buf, "xml:id", &n.xml_id);
    serialize_opt_attr(buf, "xml:lang", &n.xml_lang);
    serialize_xml_space(buf, &n.xml_space);
    serialize_xml_base(buf, &n.xml_base);
    serialize_opt_attr(buf, "condition", &n.condition);
    serialize_ns_attrs(buf, &n.foreign_attributes, ns);
    let scoped = n.scoped_namespaces.clone().unwrap_or_default();
    register_scoped(ns, &scoped);
    buf.push('>');
    buf.push_str(&xml_escape(&n.text));
    buf.push_str("</ttm:name>\n");
}

/// Serialize `<ttm:item>` — TTML2 §14.1.6.
fn serialize_ttm_item(
    item: &TtmItemElement,
    buf: &mut String,
    depth: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(depth);
    buf.push_str(&format!("{ind}<ttm:item"));
    serialize_opt_attr(buf, "name", &item.name);
    serialize_opt_attr(buf, "xml:id", &item.xml_id);
    serialize_opt_attr(buf, "xml:lang", &item.xml_lang);
    serialize_xml_space(buf, &item.xml_space);
    serialize_xml_base(buf, &item.xml_base);
    serialize_opt_attr(buf, "condition", &item.condition);
    serialize_ns_attrs(buf, &item.foreign_attributes, ns);
    let scoped = item.scoped_namespaces.clone().unwrap_or_default();
    register_scoped(ns, &scoped);

    let has_children = !item.items.is_empty() || !item.unknown_children.is_empty();
    if !has_children {
        match item.text.as_deref() {
            Some(text) if !text.is_empty() => {
                buf.push('>');
                buf.push_str(&xml_escape(text));
                buf.push_str("</ttm:item>\n");
            }
            _ => buf.push_str("/>\n"),
        }
        return;
    }
    buf.push_str(">\n");
    if let Some(text) = item.text.as_deref().filter(|t| !t.trim().is_empty()) {
        buf.push_str(&format!(
            "{}{}\n",
            "  ".repeat(depth + 1),
            xml_escape(text.trim())
        ));
    }
    for nested in &item.items {
        serialize_ttm_item(nested, buf, depth + 1, ns);
    }
    for u in &item.unknown_children {
        serialize_unknown_element(u, buf, depth + 1, ns);
    }
    buf.push_str(&format!("{ind}</ttm:item>\n"));
}

/// Serialize `<styling>` — TTML2 §10.1.3.
fn serialize_styling_element(
    styling: &StylingElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<styling"));
    serialize_opt_attr(buf, "xml:id", &styling.xml_id);
    serialize_opt_attr(buf, "xml:lang", &styling.xml_lang);
    serialize_xml_space(buf, &styling.xml_space);
    serialize_xml_base(buf, &styling.xml_base);
    serialize_ns_attrs(buf, &styling.foreign_attributes, ns);
    buf.push_str(">\n");
    for init in &styling.initials {
        let inner = "  ".repeat(indent + 1);
        buf.push_str(&format!("{inner}<initial"));
        serialize_opt_attr(buf, "xml:id", &init.xml_id);
        serialize_opt_attr(buf, "xml:lang", &init.xml_lang);
        serialize_xml_space(buf, &init.xml_space);
        serialize_xml_base(buf, &init.xml_base);
        serialize_opt_attr(buf, "condition", &init.condition);
        serialize_style_attrs(&init.style_attributes, buf);
        serialize_ns_attrs(buf, &init.foreign_attributes, ns);
        buf.push_str("/>\n");
    }
    for style in &styling.styles {
        let inner = "  ".repeat(indent + 1);
        buf.push_str(&format!("{inner}<style"));
        serialize_opt_attr(buf, "xml:id", &style.xml_id);
        serialize_opt_attr(buf, "xml:lang", &style.xml_lang);
        serialize_xml_space(buf, &style.xml_space);
        serialize_xml_base(buf, &style.xml_base);
        serialize_opt_attr(buf, "condition", &style.condition);
        serialize_opt_attr(buf, "style", &style.style);
        serialize_style_attrs(&style.style_attributes, buf);
        serialize_ns_attrs(buf, &style.foreign_attributes, ns);
        buf.push_str("/>\n");
    }
    for unknown in &styling.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</styling>\n"));
}

/// Serialize `<layout>` — TTML2 §11.1.1.
fn serialize_layout_element(
    layout: &LayoutElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<layout"));
    serialize_opt_attr(buf, "xml:id", &layout.xml_id);
    serialize_opt_attr(buf, "xml:lang", &layout.xml_lang);
    serialize_xml_space(buf, &layout.xml_space);
    serialize_xml_base(buf, &layout.xml_base);
    serialize_ns_attrs(buf, &layout.foreign_attributes, ns);
    buf.push_str(">\n");
    for region in &layout.regions {
        serialize_region_element(region, buf, indent + 1, ns);
    }
    for unknown in &layout.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</layout>\n"));
}

/// Serialize `<region>` — TTML2 §11.1.2.
fn serialize_region_element(
    region: &RegionElement,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    buf.push_str(&format!("{ind}<region"));
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
    serialize_opt_attr(buf, "ttm:roleSource", &region.ttm_role_source);
    serialize_style_attrs(&region.style_attributes, buf);
    serialize_opt_attr(buf, "xml:id", &region.xml_id);
    serialize_opt_attr(buf, "xml:lang", &region.xml_lang);
    serialize_xml_space(buf, &region.xml_space);
    serialize_xml_base(buf, &region.xml_base);
    serialize_ns_attrs(buf, &region.foreign_attributes, ns);

    let has_children = !region.metadata.is_empty()
        || !region.animations.is_empty()
        || !region.styles.is_empty()
        || !region.unknown_children.is_empty();
    if !has_children {
        buf.push_str("/>\n");
        return;
    }
    buf.push_str(">\n");
    for meta in &region.metadata {
        serialize_metadata_child(meta, buf, indent + 1, ns);
    }
    for anim in &region.animations {
        serialize_animation_child(anim, buf, indent + 1, ns);
    }
    for style in &region.styles {
        let inner = "  ".repeat(indent + 1);
        buf.push_str(&format!("{inner}<style"));
        serialize_opt_attr(buf, "xml:id", &style.xml_id);
        serialize_opt_attr(buf, "xml:lang", &style.xml_lang);
        serialize_xml_space(buf, &style.xml_space);
        serialize_xml_base(buf, &style.xml_base);
        serialize_opt_attr(buf, "condition", &style.condition);
        serialize_opt_attr(buf, "style", &style.style);
        serialize_style_attrs(&style.style_attributes, buf);
        serialize_ns_attrs(buf, &style.foreign_attributes, ns);
        buf.push_str("/>\n");
    }
    for unknown in &region.unknown_children {
        serialize_unknown_element(unknown, buf, indent + 1, ns);
    }
    buf.push_str(&format!("{ind}</region>\n"));
}

/// Serialize an animation child — TTML2 §13.1.3.
fn serialize_animation_child(
    anim: &AnimationChild,
    buf: &mut String,
    indent: usize,
    ns: &mut NamespaceMap,
) {
    let ind = "  ".repeat(indent);
    match anim {
        AnimationChild::Set(set) => {
            buf.push_str(&format!("{ind}<set"));
            serialize_common_timing_attrs(
                buf,
                set.begin.as_deref(),
                set.dur.as_deref(),
                set.end.as_deref(),
                None,
            );
            serialize_opt_attr(buf, "fill", &set.fill);
            serialize_opt_attr(buf, "repeatCount", &set.repeat_count);
            serialize_opt_attr(buf, "condition", &set.condition);
            serialize_opt_attr(buf, "ttm:role", &set.ttm_role);
            serialize_opt_attr(buf, "ttm:roleSource", &set.ttm_role_source);
            serialize_style_attrs(&set.style_attributes, buf);
            serialize_opt_attr(buf, "xml:id", &set.xml_id);
            serialize_opt_attr(buf, "xml:lang", &set.xml_lang);
            serialize_xml_space(buf, &set.xml_space);
            serialize_xml_base(buf, &set.xml_base);
            serialize_ns_attrs(buf, &set.foreign_attributes, ns);

            let has_children = !set.metadata.is_empty() || !set.unknown_children.is_empty();
            if !has_children {
                buf.push_str("/>\n");
                return;
            }
            buf.push_str(">\n");
            for meta in &set.metadata {
                serialize_metadata_child(meta, buf, indent + 1, ns);
            }
            for unknown in &set.unknown_children {
                serialize_unknown_element(unknown, buf, indent + 1, ns);
            }
            buf.push_str(&format!("{ind}</set>\n"));
        }
    }
}

/// Write the shared timing attributes — TTML2 §11.3.
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

/// Write an optional attribute, skipping absent and empty values.
fn serialize_opt_attr(buf: &mut String, name: &str, value: &Option<String>) {
    if let Some(v) = value
        && !v.is_empty()
    {
        buf.push_str(&format!(r#" {}="{}""#, name, xml_escape(v)));
    }
}

/// Basic XML escaping for attribute values and text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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
