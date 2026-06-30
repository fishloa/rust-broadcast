//! Error type returned by every parser and builder in this crate.

use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ISO/IEC 14496-12:2015 (§4.2) where applicable.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error("buffer too short: need {need} bytes, have {have} (while parsing {what})")]
    BufferTooShort {
        /// Bytes required to proceed.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// Human-readable name of the type or field being parsed.
        what: &'static str,
    },

    /// Box size was declared as 1 (triggers largesize) but fewer than 8 bytes available.
    #[error("largesize indicated but buffer too short: need {need}, have {have}")]
    LargesizeBufferTooShort {
        /// Bytes required for largesize.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },

    /// Box type was 'uuid' but fewer than the required 16 bytes of usertype available.
    #[error("uuid box indicated but buffer too short: need {need}, have {have}")]
    UuidBufferTooShort {
        /// Bytes required for usertype.
        need: usize,
        /// Bytes actually available.
        have: usize,
    },

    /// A box claimed a size smaller than its header, which is impossible.
    #[error("box size {size} is smaller than header ({header_size} bytes)")]
    BoxSizeUnderflow {
        /// Declared size.
        size: u64,
        /// Minimum header bytes.
        header_size: usize,
    },

    /// Write buffer passed to `serialize_into` was smaller than `serialized_len()`.
    #[error("serialize: output buffer too small — need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Required size.
        need: usize,
        /// Actual size.
        have: usize,
    },

    /// A field had an invalid or reserved value.
    #[error("invalid {field}: {reason} (value: 0x{value:X})")]
    InvalidValue {
        /// Name of the field.
        field: &'static str,
        /// The parsed value.
        value: u64,
        /// Human-readable explanation.
        reason: &'static str,
    },
}
