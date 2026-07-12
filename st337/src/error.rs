//! Error type for `st337` burst-preamble/burst-payload parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `st337/docs/st337.md` (SMPTE ST 337:2015 §7).

use crate::burst::DataMode;

/// Result alias for `st337` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An ST 337 burst parse / serialize error.
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
    /// `Pa` or `Pb` did not equal the fixed 16-bit-mode sync-word constant
    /// (SMPTE ST 337 Table 6, §7.2.1).
    #[error(
        "invalid {word} sync value: expected {expected:#06x}, found {found:#06x} \
         (SMPTE ST 337 Table 6)"
    )]
    InvalidSync {
        /// Which sync word (`"Pa"` or `"Pb"`).
        word: &'static str,
        /// The fixed value Table 6 requires.
        expected: u16,
        /// The value actually found.
        found: u16,
    },
    /// A field value did not fit its wire bit-width, or a derived count
    /// (`length_code` vs. actual payload length) was inconsistent.
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field/derived-count name.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// `data_mode` (§7.2.4.3 Table 8) was not `Mode16`. This crate's
    /// byte-stream abstraction only supports the 16-bit data mode — see
    /// `docs/st337.md`'s "Scope decisions" §2 for why 20-/24-bit-wide
    /// preamble words are out of scope.
    #[error(
        "data_mode {0:?} is not supported by this crate (only Mode16 -- \
         see docs/st337.md scope decision 2)"
    )]
    UnsupportedDataMode(DataMode),
    /// `data_type == 31` requires the six-word preamble (`Pe`/`Pf` present);
    /// any other `data_type` requires the four-word preamble (`Pe`/`Pf`
    /// absent) — SMPTE ST 337 §7.2.1 Table 6 / §7.2.4.2.
    #[error(
        "data_type {data_type} requires the six-word preamble (Pe/Pf) to be \
         {expected_extended}, but it was {found_extended}"
    )]
    ExtendedPreambleMismatch {
        /// The `data_type` value that determines the required form.
        data_type: u8,
        /// Whether the six-word preamble is required for this `data_type`.
        expected_extended: bool,
        /// Whether the six-word preamble was actually present.
        found_extended: bool,
    },
    /// A payload (plus, for the six-word preamble, `Pe`+`Pf`'s 32 bits) would
    /// not fit in the 16-bit-mode `length_code`'s `0..=65535`-bit range
    /// (§7.2.5).
    #[error(
        "burst_payload of {payload_bits} bits ({extended_offset_bits} extended-preamble bits \
         included) exceeds the 16-bit-mode length_code maximum of 65535 bits"
    )]
    PayloadTooLarge {
        /// Total bits that would need to be encoded in `length_code`.
        payload_bits: u32,
        /// Bits of that total contributed by `Pe`/`Pf` (0 or 32).
        extended_offset_bits: u32,
    },
}
