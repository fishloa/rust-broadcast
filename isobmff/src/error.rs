//! Error type returned by every ISO BMFF box parser and builder in this crate.
//!
//! The shared failure kinds ([`BufferTooShort`](broadcast_common::error::BufferTooShort),
//! [`OutputBufferTooSmall`](broadcast_common::error::OutputBufferTooSmall),
//! [`InvalidValue`](broadcast_common::error::InvalidValue),
//! [`InvalidField`](broadcast_common::error::InvalidField),
//! [`FieldOverflow`](broadcast_common::error::FieldOverflow),
//! [`UnsupportedVersion`](broadcast_common::error::UnsupportedVersion)) are
//! wrapped transparently so a caller can convert them through its own
//! `#[from]`; the rest are ISO BMFF-specific.

use thiserror::Error;

/// Width of a box-type four-CC.
const BOX_TYPE_LEN: usize = 4;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that box parsers and builders can return.
///
/// A new variant also needs an arm in `transmux`'s
/// `impl From<Error> for transmux::Error` and a case in
/// `transmux/tests/it/isobmff_bridge.rs`; that conversion's fallback arm would
/// otherwise swallow it silently.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ISO/IEC 14496-12:2026 (§4.2) where applicable.
// `PartialEq` but not `Eq`: kept identical to `transmux::Error`'s derives so the
// box layer converts without a bound change.
#[derive(Debug, Error, PartialEq, Clone)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than the smallest valid encoding for the type.
    #[error(transparent)]
    BufferTooShort(#[from] broadcast_common::error::BufferTooShort),

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
    #[error(transparent)]
    OutputBufferTooSmall(#[from] broadcast_common::error::OutputBufferTooSmall),

    /// A field had an invalid or reserved value.
    #[error(transparent)]
    InvalidValue(#[from] broadcast_common::error::InvalidValue),

    /// A field was missing or structurally invalid.
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),

    /// A box did not carry the four-CC the parser expected.
    #[error("unexpected box: expected {expected}")]
    UnexpectedBox {
        /// The four-CC (or description) the parser required.
        expected: &'static str,
    },

    /// A caller-supplied argument violated a documented precondition.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),

    /// The zero-width `trun`s (no per-sample fields, so the buffer length cannot
    /// bound their `sample_count`) of one `moof` — or of the run of `moof`s a
    /// demuxer holds before their `mdat` — declare more samples in total than
    /// [`MAX_ZERO_WIDTH_TRUN_SAMPLES`](crate::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES)
    /// allows. The allowance is aggregate: a 16-byte `trun` may legally declare
    /// 2^20 samples, so a per-`trun` cap would let N such `trun`s allocate
    /// N x 40 MiB.
    #[error(
        "trun sample_count with no per-sample fields exceeds MAX_ZERO_WIDTH_TRUN_SAMPLES \
         ({limit}, aggregate per moof)"
    )]
    ZeroWidthTrunBudgetExceeded {
        /// The aggregate allowance (`MAX_ZERO_WIDTH_TRUN_SAMPLES`).
        limit: usize,
    },

    /// Resolved sample data claims more bytes than the container it is being
    /// read from holds.
    ///
    /// The bound is structural, not a tunable: samples that do not overlap can
    /// never total more than the input length (the allowance is
    /// [`MAX_SAMPLE_BYTES_PER_INPUT_LENGTH`](crate::frag_offsets::MAX_SAMPLE_BYTES_PER_INPUT_LENGTH)
    /// times it). A larger total means overlapping `stco`/`stsc` chunks,
    /// overlapping `trun` runs, or several `moof`s each claiming the same
    /// `mdat` — every one of which would have its bytes copied again, turning
    /// a linear input into quadratic memory (audit 20 findings 6 and 13).
    /// Progressive layouts and fragmented demux runs share this allowance, so
    /// the per-`moof` total is not the last word.
    #[error(
        "sample data budget exceeded: resolved sample bytes exceed the {limit}-byte allowance \
         (overlapping chunks, runs or fragments?)"
    )]
    SampleDataBudgetExceeded {
        /// The aggregate allowance for this run, in bytes.
        limit: usize,
    },

    /// A length or count did not fit the wire field it is written to (#1129):
    /// a serializer that narrowed a `usize` with `as u8`/`as u16`/`as u32` (or
    /// masked it) would have silently wrapped and emitted a misframed box or
    /// section; this is returned instead.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),

    /// A count or size read from the input exceeded a configured
    /// [`Limits`](crate::Limits) cap before anything was allocated from it.
    #[error("{what} exceeds the configured limit of {limit}")]
    LimitExceeded {
        /// Which quantity exceeded its cap.
        what: &'static str,
        /// The cap it exceeded.
        limit: u64,
    },

    /// A buffered fragment would exceed
    /// [`Limits::max_fragment_bytes`](crate::Limits::max_fragment_bytes), so
    /// the writer refused the push before the buffer grew.
    #[error("fragment exceeds the configured limit of {limit} bytes")]
    FragmentTooLarge {
        /// The cap it exceeded.
        limit: u64,
    },

    /// A progressive file's `moov` follows its `mdat`, which a forward-only
    /// source cannot reach without discarding the media it has not read.
    #[error("moov follows mdat and the source is forward-only")]
    MoovAfterMdatForwardOnly,

    /// Bytes supplied to a [`Reader`](crate::reader::Reader) did not match the
    /// outstanding [`ByteRange`](crate::reader::ByteRange) demand.
    #[error("supplied bytes {got:?} do not match the outstanding need {expected:?}")]
    SupplyMismatch {
        /// The range the reader asked for.
        expected: crate::reader::ByteRange,
        /// The range the caller supplied.
        got: crate::reader::ByteRange,
    },
    /// A seek needs a `stss`, `sidx` or `tfra` index and the file carries none.
    #[error("track {track} has no random-access index to seek on")]
    NoIndex {
        /// The track that was to be sought.
        track: crate::TrackId,
    },

    /// The input needs a feature this crate does not implement yet.
    ///
    /// Distinct from a codec/scheme this crate has no handling for at all:
    /// this variant is for a *partially* supported wire feature where silently
    /// ignoring the unimplemented part would produce a plausible but wrong
    /// result rather than an obvious failure.
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(&'static str),

    /// The `FullBox` `version` was a value this parser does not implement.
    #[error(transparent)]
    UnsupportedVersion(#[from] broadcast_common::error::UnsupportedVersion),

    /// The box type at bytes `[4:8]` was not `emsg`.
    #[error("not an emsg box: type {found:?}")]
    NotEmsg {
        /// The 4-byte box type actually found.
        found: [u8; BOX_TYPE_LEN],
    },

    /// The `emsg` `size` field is inconsistent with the available bytes (it
    /// must be at least the fixed header + cover the box, and 0 / 1 large-size
    /// forms are not supported for `emsg`).
    #[error("invalid emsg box size {size}: {reason}")]
    InvalidSize {
        /// The `size` field value.
        size: u32,
        /// Why it is invalid.
        reason: &'static str,
    },

    /// A null-terminated UTF-8 string field of an `emsg` was malformed (no
    /// terminator, or invalid UTF-8).
    #[error("invalid {field} string: {reason}")]
    InvalidString {
        /// Which string field (`scheme_id_uri` / `value`).
        field: &'static str,
        /// Why it is invalid.
        reason: &'static str,
    },
}
