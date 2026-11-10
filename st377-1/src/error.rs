//! Error type for MXF (SMPTE ST 377-1) parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `private/specs-verified/smpte/st377-1/st377-1.md`.

/// Result alias for `st377-1` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An MXF parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer shorter than
    /// required.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// Output buffer smaller than the serialized form requires.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A KLV triplet or BER length token failure (SMPTE ST 336 framing, as
    /// constrained for MXF by `private/specs-verified/smpte/st377-1/st377-1.md`
    /// §6.3.4): an indefinite length, a too-long long form, or a fixed width
    /// too small for the length being serialized.
    #[error(transparent)]
    Klv(#[from] broadcast_common::klv::KlvError),

    /// A Partition Pack's Key byte 14 (Partition Kind) was not one of the
    /// three defined values (`private/specs-verified/smpte/st377-1/st377-1.md` §7.1 Table 4).
    #[error("unknown Partition Kind byte: {byte:#04X}")]
    UnknownPartitionKind {
        /// The offending byte 14 value.
        byte: u8,
    },
    /// A Partition Pack's Key byte 15 (Partition Status) was not one of
    /// the four defined values (`private/specs-verified/smpte/st377-1/st377-1.md` §7.1 Table 4 / §6.2.3).
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
    /// Primer Pack / Random Index Pack prefix (`private/specs-verified/smpte/st377-1/st377-1.md` §7.1
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
    /// match the expected fixed element size (`private/specs-verified/smpte/st377-1/st377-1.md` §4.3).
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
    /// A Local Set (§9.3) had the same local tag more than once.
    #[error("duplicate local tag {0:#06X} within a Local Set")]
    DuplicateLocalTag(u16),
    /// A Primer Pack (§9.2) mapped the same local tag to two different
    /// entries — each local tag must be unique within the Primer, or a
    /// lookup can't tell which entry is meant.
    #[error("duplicate local tag {0:#06X} within a Primer Pack")]
    DuplicatePrimerTag(u16),
    /// A Primer Pack (§9.2) mapped two different local tags to the same
    /// UL/UUID.
    #[error("duplicate UL/UUID within a Primer Pack (tags {0:#06X} and {1:#06X})")]
    DuplicatePrimerUl(u16, u16),
}
