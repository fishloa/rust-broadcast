//! Error type for VBI (ETSI EN 301 775) PES data-field parsing and serialization.

/// Result alias for VBI parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A VBI parse / serialize error.
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
    /// A `data_unit_length` field did not match the bytes the typed payload
    /// occupies (EN 301 775 §4.4, Table 1).
    #[error("invalid data_unit_length {length} for data_unit_id {id:#04X}: {reason}")]
    InvalidDataUnitLength {
        /// The `data_unit_length` value.
        length: u8,
        /// The `data_unit_id` it applied to.
        id: u8,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A field value did not fit in its wire bit-width (e.g. `line_offset`
    /// beyond 5 bits, `first_pixel_position` beyond 16 bits).
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u32,
        /// The field width on the wire.
        bits: u32,
    },
    /// A field value violated a spec constraint (e.g. `n_pixels` must be > 0,
    /// ETSI EN 301 775 §4.9.2).
    #[error("invalid field {what}: {reason}")]
    InvalidField {
        /// The field name.
        what: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
}
