//! Error types for ttml-subtitle.

extern crate alloc;

use alloc::string::String;
#[cfg(feature = "std")]
use broadcast_common::xml_chars::XmlLimit;
use thiserror::Error;

/// Result type alias for ttml-subtitle operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur during TTML/IMSC processing.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// XML parsing failed (syntax error, not well-formed).
    #[error("XML parse error: {0}")]
    XmlParse(String),

    /// The document exceeded an input bound
    /// ([`XmlLimits`](broadcast_common::xml_chars::XmlLimits)); raise it with
    /// [`Document::parse_str_with_limits`](crate::Document::parse_str_with_limits).
    #[cfg(feature = "std")]
    #[error("document exceeds the {limit} limit of {max}")]
    TooLarge {
        /// The bound that tripped.
        limit: XmlLimit,
        /// Its ceiling.
        max: usize,
    },

    /// The root element is not `<tt>` in the TTML namespace.
    #[error("expected root element <tt> in namespace http://www.w3.org/ns/ttml, got {0}")]
    NotTtmlRoot(String),

    /// A required element is missing.
    #[error("missing required element: {0}")]
    MissingElement(&'static str),

    /// A required attribute is missing.
    #[error("missing required attribute '{attr}' on element '{element}'")]
    MissingAttribute {
        /// The element name.
        element: &'static str,
        /// The attribute name.
        attr: &'static str,
    },

    /// A required attribute value is malformed.
    #[error("invalid value for attribute '{attr}' on element '{element}': {reason}")]
    InvalidAttribute {
        /// The element name.
        element: &'static str,
        /// The attribute name.
        attr: &'static str,
        /// Why the value is invalid.
        reason: String,
    },

    /// A `time-expression` could not be parsed.
    #[error("invalid time expression '{value}': {reason}")]
    InvalidTimeExpression {
        /// The raw time expression text.
        value: String,
        /// Why it's invalid.
        reason: String,
    },

    /// IMSC profile validation failed.
    #[error("IMSC validation error: {0}")]
    Validation(String),

    /// A generic constraint violation.
    #[error("{constraint}: {detail}")]
    ConstraintViolation {
        /// The constraint that was violated.
        constraint: String,
        /// Additional detail.
        detail: String,
    },
}
