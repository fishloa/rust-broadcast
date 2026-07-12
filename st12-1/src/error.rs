//! Error type for LTC codeword parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `st12-1/docs/st12-1.md` (SMPTE ST 12-1:2014 §8/§9).

/// Result alias for `st12-1` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An LTC codeword parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than the
    /// fixed 10-byte (80-bit) LTC codeword.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed/serialized.
        what: &'static str,
    },
    /// Bits 64–79 did not match the fixed synchronization word (§9.2.5,
    /// Table 5: bytes `0xFC 0xBF` under this crate's bit-to-byte packing —
    /// see `docs/st12-1.md`'s "Byte packing convention" section).
    #[error("sync word mismatch: expected {expected:02X?}, found {found:02X?} (ST 12-1 §9.2.5)")]
    SyncWordMismatch {
        /// The fixed sync word bytes (see [`crate::SYNC_WORD`]).
        expected: [u8; 2],
        /// The bytes actually found at positions 8/9 (bits 64–79).
        found: [u8; 2],
    },
    /// A time-address field (`hours`/`minutes`/`seconds`/`frames`) exceeded
    /// its valid range (§5.2/§6.2/§7.2, §9.2.1 Table 2).
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field name.
        field: &'static str,
        /// The offending value.
        value: u8,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// One of the eight 4-bit binary groups ("user bits", §8.1/Table 4) held
    /// a value outside `0x0..=0xF`.
    #[error("binary group {index} value {value:#X} invalid: must be 0x0-0xF")]
    InvalidBinaryGroup {
        /// The binary group's index, `0..8` (first..eighth, Table 4 order).
        index: usize,
        /// The offending value.
        value: u8,
    },
}
