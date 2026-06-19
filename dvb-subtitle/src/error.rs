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
}
