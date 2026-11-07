#![doc = include_str!("../README.md")]
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
#[cfg(feature = "std")]
pub use document::{
    MAX_TTML_ATTRIBUTE_BYTES, MAX_TTML_BYTES, MAX_TTML_DEPTH, MAX_TTML_ELEMENTS, default_limits,
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
