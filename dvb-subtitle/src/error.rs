//! Error type for DVB subtitle parsing.

/// Result alias for DVB subtitle parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A DVB subtitle parse or serialize error.
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
    /// The `data_identifier` was not the required `0x20`.
    #[error("bad data_identifier: {0:#04X} (expected 0x20)")]
    BadDataIdentifier(u8),
    /// The `sync_byte` was not the required `0x0F`.
    #[error("bad sync_byte: {0:#04X} (expected 0x0F)")]
    BadSyncByte(u8),
    /// The `end_of_PES_data_field_marker` was not `0xFF`.
    #[error("bad end_of_PES_data_field_marker: {0:#04X} (expected 0xFF)")]
    BadEndOfPesMarker(u8),
    /// An unrecognised or invalid segment_type was encountered.
    #[error("unknown segment_type: {0:#04X}")]
    UnknownSegmentType(u8),
    /// An unrecognised or invalid data_type in a pixel-data sub-block.
    #[error("unknown data_type: {0:#04X}")]
    UnknownDataType(u8),
    /// An unrecognised or invalid object_coding_method.
    #[error("unknown object_coding_method: {0:#02X}")]
    UnknownObjectCodingMethod(u8),
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
}
