//! Error type for VBI (ETSI EN 301 775) PES data-field parsing and serialization.

/// Result alias for VBI parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A VBI parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// The output buffer passed to `serialize_into` was too small.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
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
    /// A value did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
    /// A field value violated a spec constraint (e.g. `n_pixels` must be > 0,
    /// ETSI EN 301 775 §4.9.2).
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),
    /// A [`DataUnit`](crate::DataUnit)'s `data_unit_id` does not match the
    /// kind of its typed payload (ETSI EN 301 775 §4.4.2, Table 3), so
    /// serializing it would write a mislabelled data unit.
    #[error("data_unit_id {id:#04X} is inconsistent with the {payload} payload")]
    DataUnitIdPayloadMismatch {
        /// The `data_unit_id` byte the unit would have been written with.
        id: u8,
        /// The payload kind that was supplied.
        payload: &'static str,
    },
}
