//! Error types for ATSC 3.0 signalling parsing.

extern crate alloc;

use alloc::string::String;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced when parsing ATSC 3.0 signalling structures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer too short.
    #[error("buffer too short: need {need}, have {have} for {what}")]
    BufferTooShort {
        /// Minimum bytes required.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// What was being parsed.
        what: &'static str,
    },

    /// Write buffer passed to `serialize_into` was smaller than
    /// `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },

    /// Gzip decompression (RFC 1952) of an LLS table body failed.
    #[error("gzip decompression failed for {what}: {reason}")]
    Decompress {
        /// What was being decompressed.
        what: &'static str,
        /// The underlying `flate2`/IO error, stringified.
        reason: String,
    },

    /// XML parsing failed (syntax error, not well-formed).
    #[error("XML parse error in {what}: {reason}")]
    XmlParse {
        /// What was being parsed.
        what: &'static str,
        /// The underlying `roxmltree` error, stringified.
        reason: String,
    },

    /// A required element is missing.
    #[error("missing required element '{element}' in {what}")]
    MissingElement {
        /// What was being parsed.
        what: &'static str,
        /// The missing element's name.
        element: &'static str,
    },

    /// A required attribute is missing.
    #[error("missing required attribute '{attr}' on element '{element}'")]
    MissingAttribute {
        /// The element name.
        element: &'static str,
        /// The missing attribute's name.
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
}
