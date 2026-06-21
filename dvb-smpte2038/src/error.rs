//! Error type for SMPTE ST 2038 ANC-data parsing/serialization.

use dvb_common::bits::BitError;

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
    /// Underlying bit reader/writer error.
    #[error("bit stream error: {0}")]
    Bits(BitError),
}

impl From<BitError> for Error {
    fn from(e: BitError) -> Self {
        Error::Bits(e)
    }
}
