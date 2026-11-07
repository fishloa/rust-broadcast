//! Error type for DVB subtitle parsing.

/// Result alias for DVB subtitle parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A DVB subtitle parse or serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// Output buffer smaller than the serialized form requires.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// The `data_identifier` was not the required `0x20`.
    #[error("bad data_identifier: {0:#04X} (expected 0x20)")]
    BadDataIdentifier(u8),
    /// A fixed marker did not match (`sync_byte`, expected `0x0F`).
    #[error(transparent)]
    InvalidSync(#[from] broadcast_common::error::InvalidSync),
    /// The `end_of_PES_data_field_marker` was not `0xFF`.
    #[error("bad end_of_PES_data_field_marker: {0:#04X} (expected 0xFF)")]
    BadEndOfPesMarker(u8),
    /// An unrecognised or invalid segment_type was encountered.
    #[error("unknown segment_type: {0:#04X}")]
    UnknownSegmentType(u8),
    /// Stuffing byte was not 0x00.
    #[error("non-zero stuffing byte: {0:#04X} (expected 0x00)")]
    BadStuffingByte(u8),
    /// Segment length too large for parsed data.
    #[error("segment too large to serialize: segment_length oversized")]
    SegmentTooLarge,
    /// A field value did not fit its wire width (#1129/#1044): compared
    /// before narrowing, so an over-range value is rejected instead of
    /// silently masked/wrapped.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
    /// A `DisparityRegion` had 0 or more than 4 subregions — the wire
    /// `number_of_subregions_minus_1` field (Table 29) is 2 bits, so only
    /// 1..=4 subregions are representable.
    #[error("invalid subregion count: {0} (must be 1..=4)")]
    InvalidSubregionCount(usize),
    /// A `Subregion`'s `subregion_horizontal_position`/`subregion_width`
    /// presence didn't match the region's subregion count (Table 29: both
    /// are present for every subregion when, and only when, the region has
    /// more than one).
    #[error("subregion position presence inconsistent with subregion count")]
    SubregionPositionMismatch,
    /// A repeating-entry loop (Table 31's CLUT entry loop; Table 21's page
    /// composition region loop) had leftover bytes that don't form a whole
    /// entry, where the spec's own loop condition never leaves a remainder.
    #[error("{extra} trailing byte(s) in {what}: not a whole entry ({entry_len} bytes each)")]
    TrailingEntryBytes {
        /// What loop this was (e.g. `"alternative_CLUT_segment entries"`).
        what: &'static str,
        /// The fixed size of one entry.
        entry_len: usize,
        /// How many bytes were left over.
        extra: usize,
    },
    /// A field value does not fit the bits the wire format gives it (e.g. a
    /// reduced-range CLUT colour component wider than its 6/4/2-bit slot).
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),
}
