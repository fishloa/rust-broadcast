//! Error type for SRT packet parsing and serialization.
//!
//! Spec grounding: `draft-sharabayko-srt-01` §3 (Packet Structure).

/// Result alias for SRT packet parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An SRT packet parse / serialize error.
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
    /// The output buffer passed to `serialize_into` was too small.
    #[error("output buffer too small: need {need}, have {have}")]
    OutputBufferTooSmall {
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// A field value did not fit in its wire bit-width.
    #[error("field {what} value {value} does not fit in {bits} bits")]
    FieldTooWide {
        /// The over-wide field name.
        what: &'static str,
        /// The offending value.
        value: u64,
        /// The field width on the wire.
        bits: u32,
    },
    /// The Control Information Field length did not match any known ACK
    /// variant (Full / Small / Light — `draft-sharabayko-srt-01` §3.2.4).
    #[error("ACK CIF length {len} does not match Full (28), Small (16), or Light (4)")]
    InvalidAckLength {
        /// The CIF length actually found, in bytes.
        len: usize,
    },
    /// A NAK loss-list (`draft-sharabayko-srt-01` Appendix A) was not a whole
    /// number of 4-byte entries, or a range entry's second word had its top
    /// bit set (which would make it a nested range).
    #[error("invalid NAK loss list: {reason}")]
    InvalidLossList {
        /// Why the loss list is invalid.
        reason: &'static str,
    },
    /// A Handshake Extension block's declared length (in 4-byte units)
    /// overran the remaining CIF bytes (`draft-sharabayko-srt-01` §3.2.1).
    #[error("handshake extension length {declared} * 4 bytes overruns {remaining} remaining")]
    ExtensionOverrun {
        /// The declared `Extension Length` (in 4-byte blocks).
        declared: u16,
        /// The bytes actually remaining in the CIF.
        remaining: usize,
    },
    /// The Stream ID extension contents were not valid UTF-8 after undoing the
    /// 32-bit little-endian word storage (`draft-sharabayko-srt-01` §3.2.1.3).
    #[error("invalid Stream ID extension UTF-8")]
    InvalidStreamIdUtf8,
    /// A Key Material message (`draft-sharabayko-srt-01` §3.2.2) fixed-value
    /// field did not carry its mandated value.
    #[error("invalid key material {field}: {reason}")]
    InvalidKeyMaterial {
        /// The offending field.
        field: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A type-specific `parse` was called on bytes whose `F` bit (or Control
    /// Type) indicated the other packet kind.
    #[error("wrong packet kind: expected {expected}")]
    WrongPacketKind {
        /// What the caller expected to parse.
        expected: &'static str,
    },
    /// A field documented as reserved / must-be-zero (`draft-sharabayko-srt-01`
    /// §3.2, e.g. `Subtype` on a defined control type, or the header
    /// `Type-specific Information` word where the packet type does not use
    /// it) carried a non-zero value.
    #[error("reserved field {what} must be zero, found {value:#x}")]
    ReservedFieldNotZero {
        /// The offending field.
        what: &'static str,
        /// The non-zero value found.
        value: u64,
    },
    /// A general structural constraint (not a bit-width overflow) was
    /// violated — e.g. a length that must be a whole number of 4-byte words.
    #[error("invalid {what}: {reason}")]
    InvalidField {
        /// The offending field/value.
        what: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A Control Information Field documented as absent (Keep-Alive,
    /// Congestion Warning, Shutdown, ACKACK, Peer Error — §3.2) carried extra
    /// bytes.
    #[error("{what} has {extra} unexpected trailing byte(s)")]
    UnexpectedTrailingBytes {
        /// What was being parsed.
        what: &'static str,
        /// How many bytes were left over.
        extra: usize,
    },
}
