//! Error type for the ATSC A/331 Annex A ROUTE binary framing delta over
//! `rmt-flute`.

/// Result alias for this crate's parsing / serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed.
        what: &'static str,
    },
    /// The output buffer passed to `serialize_into` was too small.
    #[error("output buffer too small: need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// A field value did not fit in its wire bit-width.
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u64,
        /// The field width on the wire.
        bits: u32,
    },
    /// A field carried a value ROUTE (A/331 Annex A) does not permit.
    #[error("invalid field {what}: {reason}")]
    InvalidField {
        /// The field name.
        what: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// The underlying LCT/ALC/FLUTE framing (`rmt-flute`) failed to parse or
    /// serialize.
    #[error("LCT/ALC framing error: {0}")]
    Lct(#[from] rmt_flute::Error),
}
