//! Error type for `rdd29` frame/element parsing and serialization.
//!
//! Field-by-field semantics are documented in the curated spec oracle,
//! `rdd29/docs/rdd29.md` (SMPTE RDD 29:2019).

use broadcast_common::bits::BitError;

/// Result alias for `rdd29` parsing/serialization.
pub type Result<T> = core::result::Result<T, Error>;

/// An RDD 29 element/frame parse or serialize error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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
    /// A bit-level read/write (via [`broadcast_common::bits`]) failed while
    /// decoding or encoding `what`.
    #[error("bit-level error decoding/encoding {what}: {source}")]
    Bits {
        /// What was being parsed/serialized when the bit operation failed.
        what: &'static str,
        /// The underlying bit-reader/writer error.
        source: BitError,
    },
    /// A `Plex`-coded symbol's top (32-bit) escalation level itself read as
    /// all-ones (`0xFFFFFFFF`) — RDD 29 §3.4 caps encodable values at
    /// `0xFFFFFFFE`, so this value has no valid `Plex` encoding.
    #[error("Plex-coded field {field} read the unrepresentable escape value 0xFFFFFFFF")]
    PlexUnrepresentable {
        /// The field being decoded.
        field: &'static str,
    },
    /// A field value did not fit its wire bit-width, or a derived count
    /// (e.g. `ElementSize` vs. actual body length) was inconsistent.
    #[error("field {field} value {value} invalid: {reason}")]
    InvalidValue {
        /// The offending field/derived-count name.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A "Reserved (set to `X`)" field (RDD 29 gives an explicit literal
    /// value for every reserved field it defines) did not match its
    /// documented constant.
    #[error("reserved field {field} must be {expected:#x}, found {found:#x}")]
    InvalidReserved {
        /// The reserved field's name.
        field: &'static str,
        /// The documented literal value it must hold.
        expected: u64,
        /// The value actually found.
        found: u64,
    },
    /// The outermost `ReadElement()` header's `ElementID` did not match the
    /// element type expected at this parse site (e.g. [`crate::AtmosFrame`]
    /// requires `ATMOS_FRAME`, Table 1).
    #[error("expected ElementID {expected:#x}, found {found:#x}")]
    UnexpectedElementId {
        /// The `ElementID` this parse site requires.
        expected: u32,
        /// The `ElementID` actually read.
        found: u32,
    },
}

/// Attaches a `what`-context to a [`BitError`], turning it into an
/// [`Error::Bits`]. Used at every `BitReader`/`BitWriter` call site instead of
/// a context-free blanket `From` impl, so error messages always name the
/// field that was being decoded/encoded.
pub(crate) trait BitResultExt<T> {
    /// Map a [`BitError`] into [`Error::Bits`], naming `what` was being
    /// parsed/serialized.
    fn ctx(self, what: &'static str) -> Result<T>;
}

impl<T> BitResultExt<T> for core::result::Result<T, BitError> {
    fn ctx(self, what: &'static str) -> Result<T> {
        self.map_err(|source| Error::Bits { what, source })
    }
}
