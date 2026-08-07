//! Error type returned by every parser and builder in this crate.

use alloc::string::String;
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

    /// A [`CodecConfig`](crate::pipeline::CodecConfig) has no ISOBMFF/fMP4
    /// carriage in this crate (e.g. the WebM-native VP8 / Vorbis codecs, which
    /// are carried in the IR for `{WebM} → IR → {WebM}` and inspection only).
    #[error("codec {codec} has no ISOBMFF/fMP4 carriage in this crate")]
    UnsupportedCodec {
        /// The codec name (e.g. `"VP8"`, `"Vorbis"`).
        codec: &'static str,
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
    /// mapping in [`MkvMux`](crate::mkv_mux::MkvMux) (e.g. `Vvc`/`Flac`/`Ac4`/
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
    EmsgSerialize(#[from] mp4_emsg::Error),

    /// A demuxed ISOBMFF sample entry (`stsd` entry, ISO/IEC 14496-12:2015
    /// §8.5.2) describes a codec this crate has no
    /// [`CodecConfig`](crate::pipeline::CodecConfig) reconstruction for —
    /// e.g. a proprietary or as-yet-unimplemented FourCC
    /// ([`Fmp4Demux`](crate::media::Fmp4Demux), media plane step 2d). Once a
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
        track_id: u32,
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
        track_id: u32,
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
    ///
    /// Also returned by
    /// [`ProgressiveDemux`](crate::progressive_demux::ProgressiveDemux)'s
    /// [`Stage`](broadcast_common::Stage) adapter (issue B7, media plane step
    /// 2 fix wave 3) when `feed` would grow its whole-file buffer past the
    /// `max_bytes` bound supplied at construction — there this buffer has no
    /// partial-unit resync point to drop and continue from, so the caller
    /// should treat it as fatal for that `Stage` instance rather than keep
    /// feeding.
    #[error("{what} buffer exceeded its {cap}-byte cap and was dropped")]
    BufferCapExceeded {
        /// Human-readable name of the buffer that overflowed.
        what: &'static str,
        /// The configured cap, in bytes.
        cap: usize,
    },
}
