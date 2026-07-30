//! Error types for ttml-subtitle.

extern crate alloc;

use alloc::string::String;
use thiserror::Error;

/// Result type alias for ttml-subtitle operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur during TTML/IMSC processing.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// XML parsing failed (syntax error, not well-formed).
    #[error("XML parse error: {0}")]
    XmlParse(String),

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

    /// An unexpected element was found in a location where it is not valid.
    #[error("unexpected element '{name}' in namespace '{ns}'")]
    UnexpectedElement {
        /// The element local name.
        name: String,
        /// The element namespace, if any.
        ns: String,
    },

    /// An element was found that is prohibited by the content profile.
    #[error("prohibited element '{name}' for the claimed profile")]
    ProhibitedElement {
        /// The element name.
        name: String,
    },

    /// An attribute was found that is prohibited by the content profile.
    #[error("prohibited attribute '{attr}' for the claimed profile")]
    ProhibitedAttribute {
        /// The attribute name.
        attr: String,
    },

    /// A generic constraint violation.
    #[error("{constraint}: {detail}")]
    ConstraintViolation {
        /// The constraint that was violated.
        constraint: String,
        /// Additional detail.
        detail: String,
    },

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialize(String),
}
