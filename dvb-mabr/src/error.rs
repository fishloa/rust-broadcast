//! Error types for dvb-mabr.

extern crate alloc;

use alloc::string::String;
#[cfg(feature = "std")]
use broadcast_common::xml_chars::{XmlCharError, XmlLimit};
use thiserror::Error;

/// Result type alias for dvb-mabr operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur while parsing or serializing a DVB-MABR multicast
/// session configuration document (ETSI TS 103 769 V1.2.1 clause 10).
#[derive(Debug, Error, Clone)]
#[non_exhaustive]
pub enum Error {
    /// The input is not well-formed XML (the lexer refused it).
    #[cfg(feature = "std")]
    #[error("XML parse error: {0}")]
    XmlParse(#[from] quick_xml::Error),

    /// A character, entity reference, namespace prefix or attribute failed
    /// validation (see `broadcast_common::xml_chars::XmlCharError`).
    #[cfg(feature = "std")]
    #[error("XML validation error: {0}")]
    XmlInvalid(#[from] XmlCharError),

    /// The document is well-formed XML but not a valid configuration
    /// document shape.
    #[cfg(feature = "std")]
    #[error("XML structure error: {0}")]
    InvalidStructure(StructureError),

    /// The document root element is neither `MulticastServerConfiguration`
    /// nor `MulticastGatewayConfiguration` (clause 10.2.1).
    #[error(
        "expected root element MulticastServerConfiguration or MulticastGatewayConfiguration, got '{0}'"
    )]
    UnexpectedRoot(String),

    /// A required child element is missing.
    #[error("missing required element '{child}' under '{parent}'")]
    MissingElement {
        /// The parent element's local name.
        parent: &'static str,
        /// The missing child element's local name.
        child: &'static str,
    },

    /// A required attribute is missing.
    #[error("missing required attribute '{attr}' on element '{element}'")]
    MissingAttribute {
        /// The element's local name.
        element: &'static str,
        /// The missing attribute's local name.
        attr: &'static str,
    },

    /// A required attribute or element value could not be parsed as its
    /// declared XSD type.
    #[error("invalid value for attribute '{attr}' on element '{element}': '{value}' ({reason})")]
    InvalidAttribute {
        /// The element's local name.
        element: &'static str,
        /// The attribute's local name.
        attr: &'static str,
        /// The raw text that failed to parse.
        value: String,
        /// Why the value could not be parsed.
        reason: &'static str,
    },

    /// `ServiceComponentIdentifier/@xsi:type` names a type this crate does
    /// not recognize (clause 10.2.4: only `DASHComponentIdentifierType`,
    /// `HLSComponentIdentifierType`, and `GenericComponentIdentifierType`
    /// are defined).
    #[error("unknown ServiceComponentIdentifier xsi:type '{0}'")]
    UnknownComponentType(String),

    /// The document exceeded an input bound ([`XmlLimits`](broadcast_common::xml_chars::XmlLimits));
    /// raise it with `parse_str_with_limits`.
    #[cfg(feature = "std")]
    #[error("document exceeds the {limit} limit of {max}")]
    TooLarge {
        /// The bound that tripped.
        limit: XmlLimit,
        /// Its ceiling.
        max: usize,
    },

    /// A `ServiceComponentIdentifier` element carried no `xsi:type`
    /// attribute at all.
    #[error("ServiceComponentIdentifier is missing its xsi:type attribute")]
    MissingComponentType,
}

/// Why well-formed XML is not a valid configuration document shape.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructureError {
    /// A `DOCTYPE` declaration is not allowed (no entity may be declared).
    Doctype,
    /// The input ended inside an open element.
    UnexpectedEof,
    /// Character data appears outside the root element.
    CharDataOutsideRoot,
    /// The document has no root element.
    NoRoot,
    /// The document has more than one root element.
    MultipleRoots,
    /// Content follows the root element's end tag.
    ContentAfterRoot,
}

#[cfg(feature = "std")]
impl StructureError {
    /// Stable label, for matching or logging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Doctype => "doctype",
            Self::UnexpectedEof => "unexpected_eof",
            Self::CharDataOutsideRoot => "chardata_outside_root",
            Self::NoRoot => "no_root",
            Self::MultipleRoots => "multiple_roots",
            Self::ContentAfterRoot => "content_after_root",
        }
    }
}

#[cfg(feature = "std")]
broadcast_common::impl_spec_display!(StructureError);
