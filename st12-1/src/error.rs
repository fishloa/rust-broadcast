//! Error type for LTC codeword parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `private/specs-verified/smpte/st12-1/st12-1.md` (SMPTE ST 12-1:2014 §8/§9).

/// Result alias for `st12-1` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An LTC codeword parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer shorter than the
    /// fixed 10-byte (80-bit) LTC codeword.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

    /// Output buffer smaller than the serialized form requires.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// The sync word is not the fixed `0xFCBF` (ST 12-1 §9.2.5).
    #[error(transparent)]
    InvalidSync(#[from] broadcast_common::error::InvalidSync),
    /// A time-address field (`hours`/`minutes`/`seconds`/`frames`) exceeded
    /// its valid range (§5.2/§6.2/§7.2, §9.2.1 Table 2).
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),
    /// One of the eight 4-bit binary groups ("user bits", §8.1/Table 4) held
    /// a value outside `0x0..=0xF`.
    #[error("binary group {index} value {value:#X} invalid: must be 0x0-0xF")]
    InvalidBinaryGroup {
        /// The binary group's index, `0..8` (first..eighth, Table 4 order).
        index: usize,
        /// The offending value.
        value: u8,
    },
    /// A time-address field's units nibble held a value above 9: not a legal
    /// BCD digit (§9.2.1), so the tens/units split does not correspond to any
    /// real encoded value (S12-W1, #1117).
    #[error(
        "field {field} units digit {units:#X} invalid: BCD digit must be 0-9 (tens digit {tens})"
    )]
    InvalidBcdDigit {
        /// The offending field name (`hours`/`minutes`/`seconds`/`frames`).
        field: &'static str,
        /// The field's tens digit (always `0..=9` given its bit width).
        tens: u8,
        /// The offending units nibble, `0x0..=0xF`.
        units: u8,
    },
}
