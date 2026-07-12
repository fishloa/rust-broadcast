//! Error type for MXF (SMPTE ST 377-1) parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `st377-1/docs/st377-1.md`.

/// Result alias for `st377-1` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An MXF parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than
    /// required.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed/serialized.
        what: &'static str,
    },
    /// A BER length token used the reserved "unspecified length" value
    /// (`0x80` with zero following bytes), forbidden in MXF files
    /// (`docs/st377-1.md` §6.3.4).
    #[error("BER length used the reserved indefinite-length token (0x80)")]
    BerIndefiniteLength,
    /// A BER long-form length used more following bytes than an
    /// unsigned 64-bit length could ever need, or than MXF's own 9-byte
    /// cap (1 header + up to 8 length bytes, §6.3.4) allows.
    #[error("BER long-form length has {bytes} following bytes, exceeding the 8-byte cap")]
    BerLengthTooLong {
        /// Following-byte count found (or requested on serialize).
        bytes: usize,
    },
    /// A KLV Key was not the expected 16 bytes ( §6.3.8 — MXF Keys and
    /// Universal Labels are always exactly 16 bytes; this only fires when
    /// a caller-supplied buffer is short, since [`crate::klv::KlvItem`]
    /// itself always reads exactly 16).
    #[error("KLV key must be 16 bytes, found {found}")]
    InvalidKeyLength {
        /// Bytes actually available for the key.
        found: usize,
    },
    /// A Partition Pack's Key byte 14 (Partition Kind) was not one of the
    /// three defined values (`docs/st377-1.md` §7.1 Table 4).
    #[error("unknown Partition Kind byte: {byte:#04X}")]
    UnknownPartitionKind {
        /// The offending byte 14 value.
        byte: u8,
    },
    /// A Partition Pack's Key byte 15 (Partition Status) was not one of
    /// the four defined values (`docs/st377-1.md` §7.1 Table 4 / §6.2.3).
    #[error("unknown Partition Status byte: {byte:#04X}")]
    UnknownPartitionStatus {
        /// The offending byte 15 value.
        byte: u8,
    },
    /// A Footer Partition Pack (Table 8) used an Open status byte, which
    /// §7.4.1's note forbids ("Open Footer Partitions are not permitted").
    #[error("Footer Partition must not be Open (status byte {byte:#04X})")]
    OpenFooterPartition {
        /// The offending status byte.
        byte: u8,
    },
    /// A KLV Key did not match the expected fixed 13-byte Partition Pack /
    /// Primer Pack / Random Index Pack prefix (`docs/st377-1.md` §7.1
    /// Table 4 / §9.2 Table 13 / §12.1 Table 29).
    #[error("key prefix mismatch for {what}")]
    KeyPrefixMismatch {
        /// Which fixed-prefix pack was expected.
        what: &'static str,
    },
    /// A required Header Metadata property (Annex A) was absent from a
    /// typed Set's underlying [`crate::LocalSet`].
    #[error("required property {tag:#06X} ({name}) missing from {set}")]
    MissingRequiredProperty {
        /// The property's local tag.
        tag: u16,
        /// The property's spec name.
        name: &'static str,
        /// The enclosing Set's name.
        set: &'static str,
    },
    /// A property's value had a length that does not match its fixed-size
    /// wire type (e.g. a 16-byte UUID field encoded with fewer/more bytes).
    #[error("property {tag:#06X} ({name}) has invalid length {found}, expected {expected}")]
    InvalidPropertyLength {
        /// The property's local tag.
        tag: u16,
        /// The property's spec name.
        name: &'static str,
        /// Bytes found.
        found: usize,
        /// Bytes expected.
        expected: usize,
    },
    /// A Batch/Array's 8-byte header (`count: u32`, `item_len: u32`) did
    /// not agree with the actual buffer length, or `item_len` did not
    /// match the expected fixed element size (`docs/st377-1.md` §4.3).
    #[error("batch header invalid: count={count} item_len={item_len} buffer_len={buffer_len}")]
    InvalidBatchHeader {
        /// Declared element count.
        count: u32,
        /// Declared per-element length.
        item_len: u32,
        /// Actual remaining buffer length after the 8-byte header.
        buffer_len: usize,
    },
    /// A UTF-16 string property contained an invalid UTF-16 code unit
    /// sequence (unpaired surrogate).
    #[error("invalid UTF-16 in property {tag:#06X} ({name})")]
    InvalidUtf16 {
        /// The property's local tag.
        tag: u16,
        /// The property's spec name.
        name: &'static str,
    },
}
