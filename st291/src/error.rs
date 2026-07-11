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
    /// `stream_id` was not `0xBD` (private_stream_1).
    #[error("invalid stream_id: {0:#04X} (expected 0xBD private_stream_1)")]
    BadStreamId(u8),
    /// A required marker / fixed-bit pattern did not hold.
    #[error("bad fixed/marker bits in {0}")]
    BadFixedBits(&'static str),
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
    /// A field value did not fit in its wire bit-width.
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u32,
        /// The field width on the wire.
        bits: u32,
    },
    /// `user_data_words.len()` does not equal the expected UDW count
    /// (`data_count & 0xFF`); serializing would zero-fill or truncate.
    #[error("inconsistent user_data_words length: have {have}, need {need} (data_count & 0xFF)")]
    InconsistentUdwLength {
        /// Actual `user_data_words.len()`.
        have: usize,
        /// Expected count: `data_count & 0xFF`.
        need: usize,
    },
    /// A field required by the spec to be `0` was not — RFC 8331 §2.1
    /// `reserved` (22 bits) or a per-ANC-packet `word_align` padding region
    /// (`rtp` feature).
    #[error("reserved field {what} was not zero: {value:#X}")]
    ReservedNotZero {
        /// The field name (e.g. `"reserved (RFC 8331 §2.1)"` or
        /// `"word_align"`).
        what: &'static str,
        /// The nonzero value actually present.
        value: u64,
    },
    /// An RFC 8331 ANC RTP payload's declared `Length` did not match the
    /// bytes actually consumed while parsing `ANC_Count` ANC packets (`rtp`
    /// feature) — catches a corrupted `Length` or a corrupted `ANC_Count`
    /// (either one desyncs the two).
    #[error("ANC RTP payload Length mismatch: header declares {declared}, computed {computed}")]
    LengthMismatch {
        /// The `Length` value read from the wire.
        declared: usize,
        /// The number of bytes actually consumed while parsing `ANC_Count`
        /// ANC packets.
        computed: usize,
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
