//! Error type for cc_data parsing.

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// A cc_data parse error.
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
    /// Output buffer too small for serialization.
    #[error("output buffer too small: need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// More than 31 triplets — cc_count is a 5-bit field.
    #[error("too many cc triplets: {0} (cc_count is 5-bit, max 31)")]
    TooManyTriplets(usize),
}
