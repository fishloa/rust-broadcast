//! Error type for RTP fixed-header parsing/serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `rtp-packet/docs/rtp-header.md` (RFC 3550 §5.1 / §5.3.1).

/// Result alias for `rtp-packet` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An RTP header parse / serialize error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input (on parse) or output buffer (on serialize) shorter than required.
    #[error("buffer too short: need {need}, have {have} ({what})")]
    BufferTooShort {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
        /// What was being parsed/serialized.
        what: &'static str,
    },
    /// The 2-bit `version` field was not `2` (RFC 3550 §5.1: "The version
    /// defined by this specification is two (2)").
    #[error("invalid RTP version: {0} (RFC 3550 §5.1 requires version 2)")]
    InvalidVersion(u8),
    /// A field value did not fit its wire bit-width, or a derived count (CSRC
    /// list length, extension word count, padding octet count) overflowed its
    /// field.
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field/derived-count name.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// The padding region (RFC 3550 §5.1) was malformed: either the trailing
    /// count byte was zero, exceeded the bytes actually available, or (on
    /// serialize) a supplied padding slice's last byte did not equal its own
    /// length.
    #[error("invalid padding: count {count} ({reason})")]
    InvalidPadding {
        /// The padding-count byte (the last byte of the padding region).
        count: u8,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A header-extension `data` slice's length was not a multiple of 4 bytes
    /// — RFC 3550 §5.3.1's `length` field counts whole 32-bit words.
    #[error(
        "header extension data length {data_len} is not a multiple of 4 bytes \
         (RFC 3550 §5.3.1: length counts 32-bit words)"
    )]
    ExtensionNotWordAligned {
        /// The offending `data.len()`.
        data_len: usize,
    },
    /// (`rfc8285` feature) A [`HeaderExtension`](crate::HeaderExtension)'s
    /// `profile_id` matched neither the RFC 8285 one-byte form (`0xBEDE`) nor
    /// the two-byte form (`profile_id & 0xFFF0 == 0x1000`). This is **not** a
    /// malformed-packet error: RFC 8285 interpretation of the RFC 3550
    /// §5.3.1 opaque extension `data` is profile-scoped and opt-in, so an
    /// extension with some other `profile_id` is simply not one this crate's
    /// `rfc8285` decoder understands.
    #[cfg(feature = "rfc8285")]
    #[error(
        "profile_id {profile_id:#06x} matches neither the RFC 8285 one-byte \
         (0xBEDE) nor two-byte (0x1000-0x100F) header-extension form"
    )]
    NotRfc8285Extension {
        /// The offending `HeaderExtension::profile_id`.
        profile_id: u16,
    },
    /// (`rfc8285` feature) An RFC 8285 one-byte-form local identifier was
    /// outside the valid `1..=14` range (§4.2: 0 is reserved for padding, 15
    /// is reserved for a future extension / used as the "stop" marker).
    #[cfg(feature = "rfc8285")]
    #[error(
        "invalid RFC 8285 one-byte extension element id {0} (valid range is 1..=14; \
         0 is reserved for padding, 15 is reserved)"
    )]
    InvalidOneByteExtensionId(u8),
    /// (`rfc8285` feature) An RFC 8285 two-byte-form local identifier was 0
    /// (§4.1.2 / §5: "0 is reserved for padding in both forms").
    #[cfg(feature = "rfc8285")]
    #[error(
        "invalid RFC 8285 two-byte extension element id 0 (reserved for padding \
         in both forms, per RFC 8285 §4.1.2/§5)"
    )]
    InvalidTwoByteExtensionId,
}
