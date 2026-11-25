//! Error type returned by every parser and builder in this crate.

use crate::TrackId;
use alloc::string::String;
use thiserror::Error;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Error variants that parsers + builders can return.
///
/// Spec references inside `#[error(...)]` strings quote clauses from
/// ISO/IEC 14496-12:2026 (§4.2) where applicable.
// No longer `Eq` (only `PartialEq`): `Error::HlsAttrValue` wraps
// `broadcast_hls::Error`, which itself carries a non-`Eq` `f64` (issue
// #1140 T12 decimal validation).
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

    /// The XML writer refused an event while building a PlayReady
    /// WRMHEADER; the string is the writer's message.
    #[cfg(feature = "std")]
    #[error("WRMHEADER write failed: {message}")]
    Wrmheader {
        /// The writer's message.
        message: String,
    },

    /// The XML writer refused an event while rendering a manifest.
    #[cfg(feature = "std")]
    #[error(transparent)]
    ManifestWrite(#[from] broadcast_smooth::ManifestWriteError),

    /// A KLV triplet or BER length token failure (SMPTE ST 336 framing).
    #[error(transparent)]
    Klv(#[from] broadcast_common::klv::KlvError),

    /// A field was missing or structurally invalid.
    #[error(transparent)]
    InvalidField(#[from] broadcast_common::error::InvalidField),

    /// A box did not carry the four-CC the parser expected.
    #[error("unexpected box: expected {expected}")]
    UnexpectedBox {
        /// The four-CC (or description) the parser required.
        expected: &'static str,
    },

    /// A caller-supplied argument violated a documented precondition (e.g. an
    /// empty track list or a non-positive segment duration passed to the
    /// [`Segmenter`](crate::segmenter::Segmenter)).
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),

    /// A byte range supplied to [`Mp4Demux`](crate::mp4_demux::Mp4Demux) did
    /// not match the range it asked for: either the offset differed, or the
    /// supply was longer than requested. Both ranges are named so the caller
    /// can see what it got wrong.
    #[error(
        "supplied range {got_offset}+{got_len} does not match the requested \
         {expected_offset}+{expected_len}"
    )]
    InputRangeMismatch {
        /// Offset the stage asked for.
        expected_offset: u64,
        /// Length the stage asked for.
        expected_len: u64,
        /// Offset the caller supplied.
        got_offset: u64,
        /// Length the caller supplied.
        got_len: u64,
    },

    /// A sample's `dts` and `pts` differ by more than `i64`, so the
    /// composition offset cannot be represented.
    #[error("sample timestamps span more than i64")]
    TimestampSpanOverflow,

    /// A sample's composition offset (PTS - DTS) does not fit the signed
    /// 32-bit wire field `trun` declares.
    #[error("sample composition offset {offset} does not fit the 32-bit field")]
    CompositionOffsetRange {
        /// The over-range offset.
        offset: i64,
    },

    /// The zero-width `trun`s (no per-sample fields, so the buffer length cannot
    /// bound their `sample_count`) of one `moof` — or of the run of `moof`s a
    /// demuxer holds before their `mdat` — declare more samples in total than
    /// [`MAX_ZERO_WIDTH_TRUN_SAMPLES`](isobmff::movie_fragment::MAX_ZERO_WIDTH_TRUN_SAMPLES)
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
    /// [`MAX_SAMPLE_BYTES_PER_INPUT_LENGTH`](isobmff::frag_offsets::MAX_SAMPLE_BYTES_PER_INPUT_LENGTH)
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

    /// A count or size read from the input exceeded a configured
    /// [`isobmff::Limits`] cap before anything was allocated from it.
    #[error("{what} exceeds the configured limit of {limit}")]
    LimitExceeded {
        /// Which quantity exceeded its cap.
        what: &'static str,
        /// The cap it exceeded.
        limit: u64,
    },

    /// A buffered fragment would exceed
    /// [`isobmff::Limits::max_fragment_bytes`], so the writer refused the push
    /// before the buffer grew.
    #[error("fragment exceeds the configured limit of {limit} bytes")]
    FragmentTooLarge {
        /// The cap it exceeded.
        limit: u64,
    },

    /// A [`CodecConfig`](crate::pipeline::CodecConfig) has no ISOBMFF/fMP4
    /// carriage in this crate (e.g. the WebM-native VP8 / Vorbis codecs, which
    /// are carried in the IR for `{WebM} → IR → {WebM}` and inspection only).
    #[error("codec {codec} has no ISOBMFF/fMP4 carriage in this crate")]
    UnsupportedCodec {
        /// The codec name (e.g. `"VP8"`, `"Vorbis"`).
        codec: &'static str,
    },

    /// One program's set of elementary streams does not fit the PES
    /// `stream_id` families (ISO/IEC 13818-1 Table 2-22).
    ///
    /// The video family is `0xE0..=0xEF` and the audio family `0xC0..=0xDF`, so
    /// at most 16 video and 32 audio elementary streams can be numbered in one
    /// program. A 17th video stream would take `0xF0` (`ECM_stream`) and a 33rd
    /// audio stream `0xE0` — inside the video range — so the muxer refuses
    /// rather than mislabelling the stream (`TsMux` output, audit r05-W22).
    #[error("too many {family} elementary streams: {max} is the stream_id family limit")]
    TooManyElementaryStreams {
        /// The `stream_id` family that overflowed (`"video"` or `"audio"`).
        family: &'static str,
        /// The family's size (16 video, 32 audio).
        max: u8,
    },

    /// A CENC protection scheme ([`CencScheme`](broadcast_common::CencScheme))
    /// this crate has no cipher implementation for was handed to the encrypt
    /// or decrypt path.
    ///
    /// `CencScheme` is `#[non_exhaustive]` and defined in `broadcast-common`
    /// (issue #878), so a scheme ISO/IEC 23001-7 defines but this crate does
    /// not implement (`cens`, `cbc1`) can reach a cipher-dispatch site here.
    /// Those sites reject rather than fall back to a different cipher:
    /// guessing would silently produce garbage plaintext (decrypt) or
    /// content protected under a scheme the manifest does not advertise
    /// (encrypt).
    #[error("CENC scheme '{scheme}' has no cipher implementation in this crate")]
    UnsupportedCencScheme {
        /// The scheme that could not be applied.
        scheme: broadcast_common::CencScheme,
    },

    /// A [`CodecConfig`](crate::pipeline::CodecConfig) has no Matroska CodecID
    /// mapping in [`MkvMuxStage`](crate::mkv_mux::MkvMuxStage) (e.g. `Vvc`/`Flac`/`Ac4`/
    /// `MpegH`/`Mpeg2Video`/`MpegAudio`/`Dts`/`Subtitle`/`Data` — see that
    /// module's docs for the mapped set). Distinct from [`Error::UnsupportedCodec`]
    /// (that variant's message names the ISOBMFF/fMP4 carriage this variant is
    /// not about).
    #[error("codec {codec} has no Matroska CodecID mapping in this crate")]
    UnsupportedMkvCodec {
        /// The codec name (e.g. `"VVC"`, `"FLAC"`, `"Subtitle"`).
        codec: &'static str,
    },

    /// The MPEG-1/2 Program Stream framing could not be parsed
    /// ([`PsDemux`](crate::PsDemux) input — ISO/IEC 13818-1 §2.5, via `mpeg_ps`).
    #[error("program stream: {0}")]
    Ps(#[from] mpeg_ps::Error),

    /// An `emsg` (Event Message Box, ISO/IEC 23009-1 §5.10.3.3) could not be
    /// serialized (e.g. the box would exceed the 4-byte `size` field range).
    #[error("emsg serialize: {0}")]
    EmsgSerialize(isobmff::Error),

    /// A demuxed ISOBMFF sample entry (`stsd` entry, ISO/IEC 14496-12:2026
    /// §8.5.2) describes a codec this crate has no
    /// [`CodecConfig`](crate::pipeline::CodecConfig) reconstruction for —
    /// e.g. a proprietary or as-yet-unimplemented FourCC
    /// ([`Mp4Demux`](crate::mp4_demux::Mp4Demux), media plane step 2d). Once a
    /// genuine gap (not merely undecoded metadata: `stpp`/`wvtt`/`ac-4` all
    /// reconstruct), the track is rejected rather than silently dropped, so
    /// the caller learns *which* sample entry it was.
    #[error("sample entry '{fourcc}' has no CodecConfig reconstruction in this crate")]
    UnsupportedSampleEntry {
        /// The rejected sample entry's four-CC, decoded lossily as text (a
        /// FourCC is nominally ASCII but not guaranteed for an unrecognised
        /// box).
        fourcc: String,
    },

    /// A fMP4/CMAF mux entry point ([`build_init_segment`](crate::pipeline::build_init_segment)
    /// and every packager built on it — [`CmafMux`](crate::media::CmafMux),
    /// [`ProgressiveMux`](crate::progressive::ProgressiveMux),
    /// [`Segmenter`](crate::segmenter::Segmenter),
    /// [`LlSegmenter`](crate::ll_dash::LlSegmenter),
    /// [`LlHlsSegmenter`](crate::ll_hls::LlHlsSegmenter)) was given a
    /// [`CodecConfig::Data`](crate::pipeline::CodecConfig::Data) track
    /// (issue #557/#576: an opaque PMT-carried elementary stream with no
    /// ISOBMFF sample entry in this crate) — CMAF/fMP4 output cannot carry
    /// it. Names the offending track (media plane step 2d; applied uniformly
    /// to every mux entry point in media plane step-2 fix wave 1, B2-B4) so
    /// the caller can pre-filter it out (e.g. with
    /// [`Media::select_tracks_by`](crate::media::Media::select_tracks_by))
    /// rather than have it silently vanish from the output.
    #[error(
        "cannot CMAF-mux track {track_id} (PMT stream_type 0x{stream_type:02X}): \
         CodecConfig::Data has no ISOBMFF sample entry in this crate"
    )]
    UnmuxableDataTrack {
        /// The offending track's [`TrackSpec::track_id`](crate::pipeline::TrackSpec::track_id).
        track_id: TrackId,
        /// The track's preserved PMT `stream_type` (ISO/IEC 13818-1 Table 2-34).
        stream_type: u8,
    },

    /// A fMP4/CMAF mux entry point (see [`Error::UnmuxableDataTrack`] for the
    /// full list) was given a
    /// [`CodecConfig::Subtitle`](crate::pipeline::CodecConfig::Subtitle)
    /// track (B1, media plane step-2 fix wave 1): reconstructing the `stpp`/
    /// `wvtt` sample entry needs more than the format tag this crate carries
    /// (`TODO(#753)`), so CMAF/fMP4 output cannot carry it yet. Names the
    /// offending track so the caller can pre-filter it out (e.g. with
    /// [`Media::select_tracks_by`](crate::media::Media::select_tracks_by))
    /// rather than have the whole package call fail opaquely.
    #[error(
        "cannot CMAF-mux track {track_id} (subtitle format {format}): \
         CodecConfig::Subtitle has no ISOBMFF re-mux sample entry in this crate yet"
    )]
    UnmuxableSubtitleTrack {
        /// The offending track's [`TrackSpec::track_id`](crate::pipeline::TrackSpec::track_id).
        track_id: TrackId,
        /// The subtitle wire format the track carries.
        format: crate::pipeline::SubtitleFormat,
    },

    /// A streaming reassembly buffer (e.g.
    /// [`rtp_stream`](crate::rtp_stream)'s per-track access-unit buffer)
    /// grew past its configured cap while waiting for a completion signal
    /// that never arrived (a dropped final FU-A fragment, a marker bit that
    /// never comes, or any other malformed/hostile input) — issue #663 P5.2,
    /// audit-ingest #4. The partial data was dropped rather than grown
    /// without bound; the buffer's owner has already reset its internal
    /// state, so the caller may simply continue feeding new input (it will
    /// resync at the next natural boundary) or treat this as a recoverable
    /// per-connection error, at its discretion.
    #[error("{what} buffer exceeded its {cap}-byte cap and was dropped")]
    BufferCapExceeded {
        /// Human-readable name of the buffer that overflowed.
        what: &'static str,
        /// The configured cap, in bytes.
        cap: usize,
    },

    /// A length or count did not fit the wire field it is written to (#1129):
    /// a serializer that narrowed a `usize` with `as u8`/`as u16`/`as u32` (or
    /// masked it) would have silently wrapped and emitted a misframed box or
    /// section; this is returned instead.
    #[error(transparent)]
    FieldOverflow(#[from] broadcast_common::error::FieldOverflow),

    /// The input needs a feature this crate does not implement yet.
    ///
    /// Distinct from [`Error::UnsupportedCodec`]/[`Error::UnsupportedCencScheme`]
    /// (which name a specific codec/scheme this crate has no handling for at
    /// all): this variant is for a *partially* supported wire feature where
    /// silently ignoring the unimplemented part would produce a plausible but
    /// wrong result rather than an obvious failure. E.g. CENC 'seig' sample-group
    /// key rotation (ISO/IEC 23001-7 §4.1/Annex A: per-sample-group KID
    /// override via `sgpd`/`sbgp`) — decrypting such content with only the
    /// track's default `tenc.default_KID` produces garbage for every sample
    /// whose group overrides the key, with no structural signal that anything
    /// went wrong (issue #990).
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(&'static str),

    /// An `#EXT-X-KEY` attribute value (`URI`/`KEYFORMAT`/`KEYFORMATVERSIONS`,
    /// [`ExtXKey::to_tag`](crate::sample_aes::ExtXKey::to_tag)) could not be
    /// represented as an RFC 8216 §4.2 quoted-string (contains `"`, CR or
    /// LF) — issue #1140 / audit r05-W10: a key-server URL assembled from
    /// request data previously went straight into the tag unvalidated, an
    /// attribute-list-injection vector.
    #[error(transparent)]
    HlsAttrValue(#[from] broadcast_hls::Error),

    /// A trick-mode `AdaptationSet` (DASH-IF IOP §3.2.9) references a main
    /// `AdaptationSet@id` that is not present in the rendered MPD Period
    /// (ISO/IEC 23009-1:2022 Table 5, printed p. 41 — `@id` is an unsigned
    /// integer unique within the Period).
    #[error(
        "trick-mode AdaptationSet {trick_adaptation_set_id} references \
         unknown main AdaptationSet {main_adaptation_set_id}"
    )]
    TrickModeMainAdaptationSetNotFound {
        /// The `@id` of the trick-mode `AdaptationSet`.
        trick_adaptation_set_id: u32,
        /// The `main_adaptation_set_id` supplied by the caller.
        main_adaptation_set_id: u32,
    },
}

// The ISO BMFF box layer below this crate has its own error type. Map it
// variant-for-variant onto this crate's existing variants (never a single
// wrapper), so every `Error::X` match in this crate, `media-plane` and
// `dash-runtime` keeps its meaning and its message. The box layer's
// `UnsupportedVersion` and `emsg` variants arrive only through the `emsg`
// parser today, which this crate has always surfaced as `EmsgSerialize`.
impl From<isobmff::Error> for Error {
    fn from(e: isobmff::Error) -> Self {
        use isobmff::Error as I;
        match e {
            I::BufferTooShort(v) => Error::BufferTooShort(v),
            I::LargesizeBufferTooShort { need, have } => {
                Error::LargesizeBufferTooShort { need, have }
            }
            I::UuidBufferTooShort { need, have } => Error::UuidBufferTooShort { need, have },
            I::BoxSizeUnderflow { size, header_size } => {
                Error::BoxSizeUnderflow { size, header_size }
            }
            I::OutputBufferTooSmall(v) => Error::OutputBufferTooSmall(v),
            I::InvalidValue(v) => Error::InvalidValue(v),
            I::InvalidField(v) => Error::InvalidField(v),
            I::UnexpectedBox { expected } => Error::UnexpectedBox { expected },
            I::InvalidInput(s) => Error::InvalidInput(s),
            I::ZeroWidthTrunBudgetExceeded { limit } => {
                Error::ZeroWidthTrunBudgetExceeded { limit }
            }
            I::SampleDataBudgetExceeded { limit } => Error::SampleDataBudgetExceeded { limit },
            I::LimitExceeded { what, limit } => Error::LimitExceeded { what, limit },
            I::FragmentTooLarge { limit } => Error::FragmentTooLarge { limit },
            I::MoovAfterMdatForwardOnly => {
                Error::UnsupportedFeature("moov after mdat over a forward-only source")
            }
            I::SupplyMismatch { .. } => {
                Error::InvalidInput("reader supply does not match its outstanding need")
            }
            I::NoIndex { .. } => Error::UnsupportedFeature("no random-access index to seek on"),
            I::FieldOverflow(v) => Error::FieldOverflow(v),
            I::UnsupportedFeature(s) => Error::UnsupportedFeature(s),
            I::UnsupportedVersion(v) => Error::EmsgSerialize(I::UnsupportedVersion(v)),
            I::NotEmsg { found } => Error::EmsgSerialize(I::NotEmsg { found }),
            I::InvalidSize { size, reason } => {
                Error::EmsgSerialize(I::InvalidSize { size, reason })
            }
            I::InvalidString { field, reason } => {
                Error::EmsgSerialize(I::InvalidString { field, reason })
            }
            I::LayoutMismatch { .. } => Error::InvalidInput(
                "a recorded fragment layout disagrees with the samples it describes",
            ),
            I::PlanMismatch { .. } => Error::InvalidInput("planned sample size mismatch"),
            I::ReorderBufferFull { .. } => Error::InvalidInput("reorder buffer full"),
            I::PlanIncomplete { .. } => Error::InvalidInput("planned sample missing"),
            I::NoPlan { .. } => Error::InvalidInput("no plan entry"),
            _ => Error::UnsupportedFeature("unrecognised ISO BMFF box error"),
        }
    }
}
