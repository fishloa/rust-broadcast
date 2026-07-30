//! TTML2 / IMSC 1.1 timed-text subtitle parser.
//!
//! Parses W3C Timed Text Markup Language 2 (TTML2) documents and validates them
//! against IMSC 1.1 Text Profile and Image Profile constraints. Parse a document,
//! then validate it separately — the two passes are independent so callers can
//! inspect a non-conformant document before deciding whether to reject it.
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

pub mod document;
pub mod error;
pub mod time;
pub mod validation;

pub use document::{
    BodyElement, BrElement, DivElement, Document, HeadElement, ImageElement, InlineContent,
    LayoutElement, PElement, RegionElement, SpanElement, StyleAttributes, StyleElement,
    StylingElement, TtElement, XmlDeclaration,
};
pub use error::{Error, Result};
pub use time::TimeExpression;
pub use validation::{ImscVersion, Profile, ValidationError, ValidationResult, Validator};

/// Parse a TTML document from a string.
///
/// This is a convenience wrapper around [`Document::parse_str`].
pub fn parse(xml: &str) -> Result<Document> {
    Document::parse_str(xml)
}
