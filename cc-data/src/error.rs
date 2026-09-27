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
    /// A bit Table B.9 fixes to a constant value didn't have that value.
    #[error(
        "invalid fixed bits in {what}: got 0x{got:02X}, expected 0x{expected:02X} (mask 0x{mask:02X})"
    )]
    InvalidFixedBits {
        /// Which fixed field failed (`"reserved/zero_bit"`, `"one_bit/reserved"`,
        /// `"marker_bits"`).
        what: &'static str,
        /// The masked bits as received.
        got: u8,
        /// The masked bits Table B.9 requires.
        expected: u8,
        /// The mask applied before comparing.
        mask: u8,
    },
}
