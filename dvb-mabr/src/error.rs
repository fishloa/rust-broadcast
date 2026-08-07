//! Error types for dvb-mabr.

extern crate alloc;

use alloc::string::String;
use thiserror::Error;

/// Result type alias for dvb-mabr operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur while parsing or serializing a DVB-MABR multicast
/// session configuration document (ETSI TS 103 769 V1.2.1 clause 10).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The input is not well-formed XML.
    #[error("XML parse error: {0}")]
    XmlParse(String),

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

    /// A `ServiceComponentIdentifier` element carried no `xsi:type`
    /// attribute at all.
    #[error("ServiceComponentIdentifier is missing its xsi:type attribute")]
    MissingComponentType,
}
