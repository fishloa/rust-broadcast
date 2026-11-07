//! Error type for cc_data parsing.

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// A cc_data parse error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// Output buffer too small for serialization.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
    /// More than 31 triplets — cc_count is a 5-bit field.
    #[error("too many cc triplets: {0} (cc_count is 5-bit, max 31)")]
    TooManyTriplets(usize),
    /// A bit Table B.9 fixes to a constant value didn't have that value.
    #[error(transparent)]
    ReservedBits(#[from] broadcast_common::error::ReservedBits),
}
