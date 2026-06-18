//! Error type for PES parsing.

/// Result alias for PES parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A PES parse error.
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
    /// `packet_start_code_prefix` was not `0x000001`.
    #[error("invalid packet_start_code_prefix: {0:#08X} (expected 0x000001)")]
    BadStartCode(u32),
    /// A required `marker_bit` was not `1` in a PTS/DTS field.
    #[error("bad timestamp marker bit in {0}")]
    BadTimestampMarker(&'static str),
    /// The PTS/DTS leading 4-bit prefix did not match the expected `0010`/`0011`/`0001`.
    #[error("bad timestamp prefix in {0}")]
    BadTimestampPrefix(&'static str),
    /// `optional_fields` exceeds the 255-byte `PES_header_data_length` limit
    /// (an 8-bit field) — cannot be serialized.
    #[error("optional_fields too large to serialize: {0} bytes (max 255)")]
    OptionalFieldsTooLarge(usize),
}
