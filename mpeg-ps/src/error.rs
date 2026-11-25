//! Error type for MPEG Program Stream parsing.

/// Result alias for Program Stream parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// A Program Stream parse error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),
    /// Output buffer passed to `serialize_into` was smaller than
    /// `serialized_len()`.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),
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
    #[error(transparent)]
    CrcMismatch(#[from] broadcast_common::error::CrcMismatch),
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
    /// A caller-set field holds a value that would not read back as the same
    /// header (serialize -> parse must be the identity, ISO/IEC 13818-1
    /// Table 2-40 `nextbits() == '1'` stream-loop condition).
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),
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
    /// The `elementary_stream_map` loop's entries do not consume exactly the
    /// declared `elementary_stream_map_length` (ISO/IEC 13818-1 Table 2-41):
    /// the leftover bytes belong to no entry and cannot be re-serialized.
    #[error(
        "elementary_stream_map_length {declared} is not covered by its entries ({parsed} bytes)"
    )]
    ElementaryStreamMapTrailingBytes {
        /// The declared `elementary_stream_map_length` value.
        declared: usize,
        /// Bytes the parsed entries occupy.
        parsed: usize,
    },
    /// A descriptor loop ends with fewer bytes than a `descriptor()` header
    /// (ISO/IEC 13818-1 §2.6).
    #[error("descriptor header at loop offset {offset} needs {need} bytes, {have} remain")]
    DescriptorHeaderTruncated {
        /// Offset of the truncated header inside the loop.
        offset: usize,
        /// Bytes the header needs.
        need: usize,
        /// Bytes left in the loop from that offset.
        have: usize,
    },
    /// A descriptor declares a `descriptor_length` that runs past the end of
    /// its loop (ISO/IEC 13818-1 §2.6).
    #[error(
        "descriptor {tag:#04X} at loop offset {offset} declares {declared} body bytes, {have} remain"
    )]
    DescriptorBodyTruncated {
        /// Offset of the descriptor header inside the loop.
        offset: usize,
        /// Its `descriptor_tag`.
        tag: u8,
        /// Its declared `descriptor_length`.
        declared: usize,
        /// Body bytes actually left in the loop.
        have: usize,
    },
    /// PES packet parse error from `mpeg-pes`.
    #[error("PES parse error: {0}")]
    Pes(#[from] mpeg_pes::Error),
    /// A PES packet is larger than the caller's [`crate::scan::PsLimits`]
    /// cap. A conformant `PES_packet_length` is 16 bits (ISO/IEC 13818-1
    /// §2.4.3.7, Table 2-21), so a complete packet is at most
    /// `6 + 65535` bytes; a larger hold is a hostile or damaged input.
    #[error("PES packet declares {declared} payload bytes, past the cap of {max}")]
    PesTooLarge {
        /// The declared `PES_packet_length`, or 0 for the unbounded form.
        declared: usize,
        /// The caller's cap.
        max: usize,
    },

    /// A length or count field's value does not fit its wire field
    /// (see `broadcast_common::len`, #1129).
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::len::FieldOverflow),
}
