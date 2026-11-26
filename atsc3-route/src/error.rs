//! Error type for the ATSC A/331 Annex A ROUTE binary framing delta over
//! `rmt-flute`.

/// Result alias for this crate's parsing / serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// A parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_core::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_core::error::OutputBufferTooSmall),
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_core::error::FieldOverflow),
    /// A field carried a value ROUTE (A/331 Annex A) does not permit.
    #[error(transparent)]
    InvalidField(#[from] broadcast_core::error::InvalidField),
    /// The underlying LCT/ALC/FLUTE framing (`rmt-flute`) failed to parse or
    /// serialize.
    #[error("LCT/ALC framing error: {0}")]
    Lct(#[from] rmt_flute::Error),
}
