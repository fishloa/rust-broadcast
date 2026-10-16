//! TTML2 / IMSC 1.1 timed-text subtitle parser.
//!
//! Parses W3C Timed Text Markup Language 2 (TTML2) documents and validates them
//! against IMSC 1.1 Text Profile and Image Profile constraints. Parse a document,
//! then validate it separately — the two passes are independent so callers can
//! inspect a non-conformant document before deciding whether to reject it.
//!
//! ## What it does / does not
//!
//! Does: parse a TTML2 document into a typed tree ([`Document`]), serialize it
//! back, and check a documented subset of the IMSC 1.1 Text/Image Profile
//! constraints ([`Validator`]). Does not: render or lay out subtitles, resolve
//! style inheritance, certify full IMSC conformance (`valid: true` only means
//! the implemented checks passed), convert to WebVTT/SRT, or decode image
//! payloads. IMSC 1.2 is not implemented.
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | W3C TTML2 | Recommendation 08 Nov 2018 | element tree, `tts:*` style attributes, §9 resource elements | Partial (unmodelled content kept as [`UnknownElement`] / [`ForeignAttribute`]) |
//! | W3C TTML2 | same | `<time-expression>` grammar ([`TimeExpression`]) | Full |
//! | W3C IMSC 1.1 | Recommendation 08 Nov 2018, edited 27 Apr 2020 | Text and Image Profile checks listed in the [`validation`] module | Partial (159-row feature table transcribed in `docs/imsc11-profiles.md`, not enforced) |
//! | W3C IMSC 1.0 / 1.0.1 | [`ImscVersion::V1_0`] | same checker minus the IMSC 1.1-only frame-rate rule | Partial |
//! | XML 1.0 | §2.8, §4.3.3 | declaration validated at parse | Full |
//!
//! ## Design
//!
//! Parse and validate are independent passes. XML I/O is `quick-xml` only; the
//! serializer is generated from typed fields. Public enums are
//! `#[non_exhaustive]`. MSRV 1.95.0, MIT OR Apache-2.0. Related crates:
//! `broadcast-common` (dependency), `caption-convert` (documents TTML source
//! conversion as a gap), `dvb-subtitle` (bitmap subtitles, a different format).
//!
//! ## Features
//!
//! XML parsing and serialization read and write through `quick-xml`, which needs
//! `std`, so [`Document`], the foreign-content types and the profile validator
//! are gated behind the (default) `std` feature. A `no_std` build exposes only
//! [`error`] and the [`time`] expression parser.
//!
//! ## Round-trip guarantee
//!
//! Parsing, serializing, and re-parsing yields a semantically equal document
//! (`parse → serialize → re-parse → equal`). The serialized XML is **not**
//! byte-identical to the input: comments, processing instructions, and XML
//! declarations are not stored, and attribute order is deterministic but not
//! preserved. Foreign content is **not** lost: attributes in namespaces the
//! crate does not model are kept as [`ForeignAttribute`] triples with their
//! original `xmlns:` prefix bindings, and unmodeled child elements (TTML2
//! §7.2/§7.3, plus any TTML2 element not given a typed struct) are kept as
//! [`UnknownElement`] subtrees on the carrying element and re-emitted with
//! the same prefix bindings, falling back to a generated `ttmfallbackN`
//! prefix only when the original one is unavailable.
//! See the crate README's "Round-Trip Guarantee" section for the full verified list.
//!
//! ## From-scratch authoring
//!
//! All element types implement `Default`. Construct a document from nothing:
//!
//! ```
//! use ttml_subtitle::{Document, InlineContent};
//!
//! let mut doc = Document::default();
//! doc.tt.xml_lang = Some("en".into());
//!
//! let mut body = ttml_subtitle::BodyElement::default();
//! let mut div = ttml_subtitle::DivElement::default();
//! let mut p = ttml_subtitle::PElement::default();
//! p.begin = Some("0s".into());
//! p.end = Some("5s".into());
//! p.content.push(InlineContent::Text("Hello".into()));
//! div.paragraphs.push(p);
//! body.divs.push(div);
//! doc.tt.body = Some(body);
//!
//! let xml = doc.to_xml();
//! assert!(xml.contains("Hello"));
//! ```
//!
//! Spec citations:
//! - W3C TTML2 Recommendation (08 Nov 2018): `ttml2-syntax.md` in this crate's `docs/`.
//! - W3C IMSC 1.1 Recommendation (08 Nov 2018, edited 27 Apr 2020): `imsc11-profiles.md` in this crate's `docs/`.
//!
//! ```
//! use ttml_subtitle::Document;
//!
//! let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
//! <tt xml:lang="en" xmlns="http://www.w3.org/ns/ttml"
//!    xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
//!    ttp:contentProfiles="http://www.w3.org/ns/ttml/profile/imsc1.1/text">
//!   <body><div><p begin="0s" end="5s">Hello</p></div></body>
//! </tt>"#;
//!
//! let doc = Document::parse_str(xml).unwrap();
//! let body = doc.tt.body.as_ref().unwrap();
//! assert_eq!(body.divs[0].paragraphs[0].begin.as_deref(), Some("0s"));
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(feature = "std")]
pub mod document;
pub mod error;
#[cfg(feature = "std")]
pub mod foreign;
#[cfg(feature = "std")]
mod pull;
pub mod time;
#[cfg(feature = "std")]
pub mod validation;

#[cfg(feature = "std")]
pub use document::{
    AudioElement, BodyElement, BrElement, ChunkElement, DataElement, DivElement, Document,
    FontElement, HeadElement, ImageElement, InlineContent, LayoutElement, PElement, RegionElement,
    ResourcesElement, SetElement, SourceElement, SpanElement, StyleAttributes, StyleElement,
    StylingElement, TtElement, XmlDeclaration,
};
pub use error::{Error, Result};
#[cfg(feature = "std")]
pub use foreign::{ForeignAttribute, UnknownElement, UnknownNode};
pub use time::TimeExpression;
#[cfg(feature = "std")]
pub use validation::{ImscVersion, Profile, ValidationError, ValidationResult, Validator};

/// Parse a TTML document from a string.
///
/// This is a convenience wrapper around [`Document::parse_str`].
#[cfg(feature = "std")]
pub fn parse(xml: &str) -> Result<Document> {
    Document::parse_str(xml)
}
