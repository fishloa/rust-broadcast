//! Error type returned by every ISO BMFF box parser and builder in this crate.
//!
//! The shared failure kinds ([`BufferTooShort`](broadcast_core::error::BufferTooShort),
//! [`OutputBufferTooSmall`](broadcast_core::error::OutputBufferTooSmall),
//! [`InvalidValue`](broadcast_core::error::InvalidValue),
//! [`InvalidField`](broadcast_core::error::InvalidField),
//! [`FieldOverflow`](broadcast_core::error::FieldOverflow),
//! [`UnsupportedVersion`](broadcast_core::error::UnsupportedVersion)) are
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
    BufferTooShort(#[from] broadcast_core::error::BufferTooShort),

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
    OutputBufferTooSmall(#[from] broadcast_core::error::OutputBufferTooSmall),

    /// A field had an invalid or reserved value.
    #[error(transparent)]
    InvalidValue(#[from] broadcast_core::error::InvalidValue),

    /// A field was missing or structurally invalid.
    #[error(transparent)]
    InvalidField(#[from] broadcast_core::error::InvalidField),

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
        limit: u64,
    },

    /// A length or count did not fit the wire field it is written to (#1129):
    /// a serializer that narrowed a `usize` with `as u8`/`as u16`/`as u32` (or
    /// masked it) would have silently wrapped and emitted a misframed box or
    /// section; this is returned instead.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_core::error::FieldOverflow),

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
    UnsupportedVersion(#[from] broadcast_core::error::UnsupportedVersion),

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

    /// A sample pushed to the progressive writer is a different length than its
    /// plan entry recorded, so it would not land where the tables say.
    #[error("track {track} sample {index} is {got} bytes, the plan says {planned}")]
    PlanMismatch {
        /// The track of the pushed sample.
        track: crate::TrackId,
        /// Zero-based sample index.
        index: u32,
        /// The planned size.
        planned: u32,
        /// The supplied size.
        got: u64,
    },

    /// A fragment written under a recorded layout would contradict it: the
    /// layout states a field once (or omits it in favour of a default) and the
    /// samples say something else, so writing it would misdescribe them.
    #[error(
        "{field} of track {track} sample {sample} is {actual}, the recorded layout states {expected}"
    )]
    LayoutMismatch {
        /// Which `tfhd`/`trun` field disagrees.
        field: &'static str,
        /// The track of the sample.
        track: crate::TrackId,
        /// Zero-based sample index inside the fragment.
        sample: u32,
        /// The value the layout states.
        expected: u64,
        /// The value the samples state.
        actual: u64,
    },

    /// An out-of-turn sample would grow the append-only sink's reorder buffer
    /// past the cap the caller chose.
    #[error("reorder buffer would exceed the caller's {cap}-byte cap")]
    ReorderBufferFull {
        /// The caller's cap, in bytes.
        cap: u64,
    },

    /// A planned sample was never pushed when the writer was finished.
    #[error("track {track} sample {missing} was planned but never pushed")]
    PlanIncomplete {
        /// The track with the gap.
        track: crate::TrackId,
        /// The first sample index that was never pushed.
        missing: u32,
    },

    /// The plan has no entry for what the caller asked for: an unknown track,
    /// a sample index past the plan, or `Interleave::AsPlanned` over a plan
    /// built without a source chunk table.
    #[error("no plan entry for {what}")]
    NoPlan {
        /// What the plan could not supply.
        what: &'static str,
    },

    /// A [`BoxRegistry`](crate::registry::BoxRegistry) registration named a
    /// key that is already registered, or a four-CC this crate types itself,
    /// and override was not requested.
    #[error("box type cannot be registered: {reason}")]
    BoxTypeRegistered {
        /// Why the registration was refused.
        reason: &'static str,
    },

    /// An `ipma` association named a property index past the `ipco` container
    /// that holds it (ISO/IEC 14496-12:2026 §8.11.14.3: `property_index` "shall
    /// not be greater than the number of boxes").
    #[error("item property index {index} is past the {available}-property container")]
    PropertyIndexOutOfRange {
        /// The 1-based index the association named.
        index: u16,
        /// How many properties the container holds.
        available: usize,
    },

    /// A derived image item's reference chain returned to an item it already
    /// visited (ISO/IEC 14496-12:2026 §8.15.3.3 requires cycles be avoided).
    #[error("derived image item {item} refers back to itself through its inputs")]
    ReferenceCycle {
        /// The item the cycle was detected at.
        item: u32,
    },

    /// An item extent addressed bytes past the end of the file it was resolved
    /// against (ISO/IEC 14496-12:2026 §8.11.3.2.4).
    #[error("item {item} extent [{offset}, +{length}) is outside the {file_len}-byte file")]
    ExtentOutOfRange {
        /// The item whose extent it is.
        item: u32,
        /// The extent's first byte.
        offset: u64,
        /// The extent's declared length.
        length: u64,
        /// The file length the extent was checked against.
        file_len: u64,
    },

    /// An item's `iloc` entry names a data reference other than "this file";
    /// this crate does not open a second file.
    #[error("item {item} names data_reference_index {index}, which is not the same file")]
    DataReferenceUnsupported {
        /// The item whose data lives elsewhere.
        item: u32,
        /// The `data_reference_index` it named.
        index: u16,
    },
}

/// Why a box the reader could not type was kept as an opaque child instead of
/// failing the whole file.
///
/// One token per [`enum@Error`] class a typed box parse can report, so a finding
/// names the fault without carrying the error's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum OpaqueReason {
    /// The box was shorter than its own syntax requires.
    BufferTooShort,
    /// A field held a value the clause does not allow.
    InvalidValue,
    /// A call supplied something the box cannot represent.
    InvalidInput,
    /// The FullBox version is one this crate does not model.
    UnsupportedVersion,
    /// The box was not the type the parser expected.
    UnexpectedBox,
    /// A count exceeded the caller's limit.
    LimitExceeded,
    /// A fault outside the classes above.
    Other,
}

impl OpaqueReason {
    /// The reason class one parse error belongs to.
    #[must_use]
    pub const fn of(error: &Error) -> Self {
        match error {
            Error::BufferTooShort(_)
            | Error::LargesizeBufferTooShort { .. }
            | Error::UuidBufferTooShort { .. } => Self::BufferTooShort,
            Error::InvalidValue(_) => Self::InvalidValue,
            Error::InvalidInput(_) => Self::InvalidInput,
            Error::UnsupportedVersion(_) => Self::UnsupportedVersion,
            Error::UnexpectedBox { .. } => Self::UnexpectedBox,
            Error::LimitExceeded { .. } => Self::LimitExceeded,
            _ => Self::Other,
        }
    }

    /// The reason's stable token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BufferTooShort => "buffer-too-short",
            Self::InvalidValue => "invalid-value",
            Self::InvalidInput => "invalid-input",
            Self::UnsupportedVersion => "unsupported-version",
            Self::UnexpectedBox => "unexpected-box",
            Self::LimitExceeded => "limit-exceeded",
            Self::Other => "other",
        }
    }
}

broadcast_core::impl_spec_display!(OpaqueReason);
