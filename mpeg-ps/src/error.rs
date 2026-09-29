//! Error type for MPEG Program Stream parsing.

/// Result alias for Program Stream parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A Program Stream parse error.
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
    /// `pack_start_code` was not `0x000001BA`.
    #[error("invalid pack_start_code: {0:#010X} (expected 0x000001BA)")]
    BadPackStartCode(u32),
    /// `system_header_start_code` was not `0x000001BB`.
    #[error("invalid system_header_start_code: {0:#010X} (expected 0x000001BB)")]
    BadSystemHeaderStartCode(u32),
    /// `map_stream_id` was not `0xBC`.
    #[error("invalid map_stream_id: {0:#04X} (expected 0xBC)")]
    BadMapStreamId(u8),
    /// Invalid PSM CRC_32.
    #[error("PSM CRC_32 mismatch: computed {computed:#010X}, stored {stored:#010X}")]
    BadCrc {
        /// CRC computed over the PSM bytes.
        computed: u32,
        /// CRC stored in the PSM trailer.
        stored: u32,
    },
    /// A required `marker_bit` was not `1`.
    #[error("bad marker bit in {0}")]
    BadMarker(&'static str),
    /// The `01` prefix after `pack_start_code` was not `01`.
    #[error("bad SCR prefix: {0:#04b} (expected 0b01)")]
    BadScrPrefix(u8),
    /// The pack header carries the ISO/IEC 11172-1 (MPEG-1) `'0010'` pack
    /// prefix, a different (12-byte, no SCR extension) layout this crate
    /// does not parse — distinct from [`Self::BadScrPrefix`], which means
    /// the bytes are neither a valid MPEG-1 nor MPEG-2 pack header (#1119).
    #[error(
        "MPEG-1 (ISO/IEC 11172-1) pack header is not supported (only ISO/IEC 13818-1 MPEG-2 packs are)"
    )]
    Mpeg1NotSupported,
    /// `system_header` `stream_id == 0xB7` but the extension form prefix is wrong.
    #[error("bad stream_id_extension prefix byte: {0:#04X}")]
    BadStreamIdExtensionPrefix(u8),
    /// `program_mux_rate` is 0 (forbidden).
    #[error("program_mux_rate is 0 (forbidden)")]
    ZeroMuxRate,
    /// `pack_stuffing_length` exceeds 7 (3-bit field).
    #[error("pack_stuffing_length {0} exceeds maximum 7")]
    StuffingLengthTooLarge(u8),
    /// `PackHeader::stuffing.len()` does not match `stuffing_length` — the
    /// two are independently-settable public fields (#1119 W8).
    #[error("pack_header stuffing_length ({declared}) does not match stuffing.len() ({actual})")]
    StuffingLengthMismatch {
        /// The `stuffing_length` field value.
        declared: u8,
        /// The actual `stuffing.len()`.
        actual: usize,
    },
    /// `header_length` overflows the available buffer.
    #[error("header_length {header_length} exceeds available bytes ({available})")]
    HeaderLengthOverflow {
        /// The header_length value.
        header_length: usize,
        /// Bytes available after the header_length field.
        available: usize,
    },
    /// `program_stream_map_length` overflows the available buffer.
    #[error("program_stream_map_length {map_length} exceeds available bytes ({available})")]
    MapLengthOverflow {
        /// The program_stream_map_length value.
        map_length: usize,
        /// Bytes available after the length field.
        available: usize,
    },
    /// PES packet parse error from `mpeg-pes`.
    #[error("PES parse error: {0}")]
    Pes(#[from] mpeg_pes::Error),

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_common::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
}
