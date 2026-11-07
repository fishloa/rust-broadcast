//! Error type for ST 291-1 ANC-data parsing/serialization (shared by every
//! transport: ST 2038 MPEG-2 TS/PES and RFC 8331 RTP).

use broadcast_common::bits::BitError;

/// Result alias for ST 2038 parsing.
pub type Result<T> = core::result::Result<T, Error>;

/// An ST 2038 parse / serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input shorter than required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// Output buffer smaller than the serialized form requires.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// `packet_start_code_prefix` was not `0x000001`.
    #[error(transparent)]
    InvalidSync(#[from] broadcast_common::error::InvalidSync),
    /// `stream_id` was not `0xBD` (private_stream_1).
    #[error("invalid stream_id: {0:#04X} (expected 0xBD private_stream_1)")]
    BadStreamId(u8),
    /// A required marker / fixed-bit pattern did not hold, or a reserved
    /// field was not zero.
    #[error(transparent)]
    ReservedBits(#[from] broadcast_common::error::ReservedBits),
    /// `PTS_DTS_flags` was not `'10'` (ST 2038 mandates PTS-only).
    #[error("invalid PTS_DTS_flags: {0:#04b} (expected 0b10, PTS only)")]
    BadPtsDtsFlags(u8),
    /// `PES_header_data_length` was not `0x05` (ST 2038 mandates exactly a PTS).
    #[error("invalid PES_header_data_length: {0:#04X} (expected 0x05)")]
    BadHeaderDataLength(u8),
    /// `descriptor_tag` was not `0xC4`.
    #[error("invalid anc_data_descriptor tag: {0:#04X} (expected 0xC4)")]
    BadDescriptorTag(u8),
    /// `PES_packet_length` overflows the available buffer.
    #[error("PES_packet_length {len} exceeds available bytes ({available})")]
    PesLengthOverflow {
        /// The `PES_packet_length` value.
        len: usize,
        /// Bytes available after the length field.
        available: usize,
    },
    /// A serialized structure exceeded a 16-bit length field.
    #[error("PES_packet_length {0} exceeds the 16-bit field maximum (65535)")]
    PesLengthTooLarge(usize),
    /// A value did not fit in its wire bit-width.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),
    /// `user_data_words.len()` does not equal the expected UDW count
    /// (`data_count & 0xFF`); serializing would zero-fill or truncate.
    #[error("inconsistent user_data_words length: have {have}, need {need} (data_count & 0xFF)")]
    InconsistentUdwLength {
        /// Actual `user_data_words.len()`.
        have: usize,
        /// Expected count: `data_count & 0xFF`.
        need: usize,
    },
    /// An RFC 8331 ANC RTP payload's declared `Length` did not match the
    /// bytes actually consumed while parsing `ANC_Count` ANC packets (`rtp`
    /// feature) — catches a corrupted `Length` or a corrupted `ANC_Count`
    /// (either one desyncs the two).
    #[error(transparent)]
    LengthMismatch(#[from] broadcast_common::error::LengthMismatch),
    /// [`crate::anc_content::AncContent::verify_checksum`] found the stored
    /// `Checksum_Word` did not match the ST 291-1 computation over the
    /// current `did`/`sdid`/`data_count`/`user_data_words` (see
    /// `private/specs-verified/smpte/st291/anc_packet_291.md` "Checksum_Word computation").
    #[error("Checksum_Word mismatch: stored {stored:#05X}, computed {computed:#05X}")]
    ChecksumMismatch {
        /// The `Checksum_Word` actually stored in the content.
        stored: u16,
        /// The value ST 291-1's checksum computation gives for the other
        /// fields.
        computed: u16,
    },
    /// [`crate::AncContent::verify_parity`] found a `DID`/`SDID`/`Data_Count`
    /// word whose `b8` is not the even parity of `b7..b0` or whose `b9` is
    /// not `NOT b8` (ST 291-1:2011 §6.1, §6.2, §6.5).
    #[error("{what} word {word:#05X} has wrong ST 291-1 parity bits (b8 even parity, b9 = NOT b8)")]
    ParityMismatch {
        /// Which word (`"DID"`, `"SDID"` or `"Data_Count"`).
        what: &'static str,
        /// The offending 10-bit word.
        word: u16,
    },
    /// Underlying bit reader/writer error.
    #[error("bit stream error: {0}")]
    Bits(BitError),
    /// The RFC 3550 RTP fixed header itself (via `rtp_packet::RtpPacket`) was
    /// malformed, when composing/decomposing a full ANC-over-RTP packet
    /// (`rtp` feature).
    #[cfg(feature = "rtp")]
    #[error("RTP header error: {0}")]
    Rtp(rtp_packet::Error),
}

impl From<BitError> for Error {
    fn from(e: BitError) -> Self {
        Error::Bits(e)
    }
}
