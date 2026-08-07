//! Matroska (MKV) container muxer — [`MkvMux`].
//!
//! [`MkvMux`] packages the crate's [`Media`] IR into a Matroska (`.mkv`) file:
//! an EBML header, then a `Segment` carrying `SeekHead` / `Info` / `Tracks` /
//! one or more `Cluster`s of `SimpleBlock`s / `Cues`. It is the exact inverse
//! of [`crate::webm_demux::WebmDemux`] — the two share the same CodecID ⇄
//! [`CodecConfig`] mapping, so `{Matroska} → IR → {Matroska}` round-trips.
//!
//! # Spec citations
//!
//! - **EBML framing** (VINT element-ID / element-size, EBML header): RFC 8794
//!   §4 / §11.
//! - **Matroska element IDs / semantics** (Segment, SeekHead, Info, Tracks,
//!   Cluster, (Simple)Block, Cues): RFC 9559 §12 / §27.
//! - **CodecID registry** (`V_MPEG4/ISO/AVC`, `V_MPEGH/ISO/HEVC`, `V_VP9`,
//!   `V_VP8`, `V_AV1`, `A_AAC`, `A_OPUS`, `A_VORBIS`, `A_AC3`, `A_EAC3`) and the
//!   `CodecPrivate` payload each one carries: RFC 9559 §27 codec mapping
//!   registry (avcC/hvcC/av1C bodies verbatim; `AudioSpecificConfig` for AAC;
//!   `OpusHead` per RFC 7845 §5.1; the Xiph-laced Vorbis headers).
//!
//! # Scope
//!
//! The mapped [`CodecConfig`] variants are exactly [`crate::webm_demux::WebmDemux`]'s
//! supported set plus the ISOBMFF-family codecs a DVR recording actually
//! carries: [`CodecConfig::Avc`], [`CodecConfig::Hevc`], [`CodecConfig::Vp9`],
//! [`CodecConfig::Vp8`], [`CodecConfig::Av1`], [`CodecConfig::Aac`],
//! [`CodecConfig::Opus`], [`CodecConfig::Vorbis`], [`CodecConfig::Ac3`],
//! [`CodecConfig::Eac3`]. Every other variant (`Vvc`/`Flac`/`Ac4`/`MpegH`/
//! `Mpeg2Video`/`MpegAudio`/`Dts`/`Subtitle`/`Data`) is rejected with
//! [`Error::UnsupportedMkvCodec`], naming the track, rather than silently
//! dropped.
//!
//! # Cluster strategy
//!
//! Samples from every track are merged into one block stream by a **k-way
//! merge on presentation time that never reorders a track's own samples**:
//! each track's `samples` are already in decode order (the IR's documented
//! invariant), and for AVC/HEVC with B-frame reordering that order is *not*
//! presentation-time order (a P-frame decodes before, but may present after,
//! the B-frames that reference it) — a track's own presentation-time sequence
//! can therefore dip non-monotonically, and a global sort on presentation time
//! would silently reorder a track's blocks relative to each other, desyncing
//! decode order from reference order. The merge only decides *when* to
//! interleave tracks, walking each track's own cursor strictly forward.
//!
//! The merged stream is then split into `Cluster`s: a new `Cluster` starts at
//! every video keyframe (when the [`Media`] has a video track), and in any
//! case at least every `MAX_CLUSTER_SPAN_WITH_VIDEO_TICKS` (5 s) — or
//! `MAX_CLUSTER_SPAN_AUDIO_ONLY_TICKS` (1 s) for an audio-only [`Media`].
//! Every block is emitted as a `SimpleBlock` (no lacing, no `BlockGroup`) —
//! valid per RFC 9559 §12 and sufficient for every codec this module maps
//! (the signed relative timestamp already accommodates the occasional small
//! backward dip from B-frame reordering).
//!
//! `TimestampScale` is fixed at `TIMESTAMP_SCALE_NS` (1 ms per tick, the
//! Matroska default) — every track's samples are rescaled from their own
//! [`TrackSpec::timescale`](crate::pipeline::TrackSpec::timescale) into that
//! common tick.
//!
//! # `SeekHead` / `Cues`
//!
//! `SeekHead` indexes `Info` and `Tracks` only (both always precede every
//! `Cluster`, so no seek entry ever needs to name one). Its `SeekPosition`
//! entries are written at a fixed 4-byte width (giving each entry's own byte
//! length independence from the position *value*, not just its presence) so
//! the `SeekHead`'s own serialized size — needed to compute those very
//! positions — can be measured from a placeholder build before the real
//! positions are known, mirroring
//! [`crate::progressive::ProgressiveMux`]'s two-pass `stco`/`co64` offset
//! arithmetic. `Cues` carries one `CuePoint` per **video** keyframe (never one
//! per audio sample, which — unlike video — has no non-keyframe state in this
//! IR: [`crate::pipeline::Sample::from_raw`] always marks an audio sample
//! sync); omitted entirely when the [`Media`] has no video track.

use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::{Package, Serialize};

use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::opus::OpusSpecificBox;
use crate::pipeline::CodecConfig;

// ---------------------------------------------------------------------------
// EBML header elements (RFC 8794 §11)
// ---------------------------------------------------------------------------

const ID_EBML: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
const ID_EBML_VERSION: [u8; 2] = [0x42, 0x86];
const ID_EBML_READ_VERSION: [u8; 2] = [0x42, 0xF7];
const ID_EBML_MAX_ID_LENGTH: [u8; 2] = [0x42, 0xF2];
const ID_EBML_MAX_SIZE_LENGTH: [u8; 2] = [0x42, 0xF3];
const ID_DOC_TYPE: [u8; 2] = [0x42, 0x82];
const ID_DOC_TYPE_VERSION: [u8; 2] = [0x42, 0x87];
const ID_DOC_TYPE_READ_VERSION: [u8; 2] = [0x42, 0x85];

/// `EBMLVersion` value this writer emits.
const EBML_VERSION_VALUE: u64 = 1;
/// `EBMLReadVersion` value this writer emits.
const EBML_READ_VERSION_VALUE: u64 = 1;
/// `EBMLMaxIDLength` value this writer emits (the widest element ID it uses is
/// 4 bytes, but 4 is also the registry-wide max for a Matroska/WebM ID).
const EBML_MAX_ID_LENGTH_VALUE: u64 = 4;
/// `EBMLMaxSizeLength` value this writer emits (matches [`encode_vint_size`]'s
/// widest emitted size VINT).
const EBML_MAX_SIZE_LENGTH_VALUE: u64 = 8;
/// `DocType` value: this writer always emits the Matroska doc type (not the
/// `webm` subset profile), since it uses codecs (AVC/HEVC/AAC/AC-3) outside
/// that profile.
const DOC_TYPE: &[u8] = b"matroska";
/// `DocTypeVersion` value this writer emits.
const DOC_TYPE_VERSION_VALUE: u64 = 4;
/// `DocTypeReadVersion` value this writer emits.
const DOC_TYPE_READ_VERSION_VALUE: u64 = 2;

// ---------------------------------------------------------------------------
// Segment + children (RFC 9559 §27)
// ---------------------------------------------------------------------------

const ID_SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
const ID_SEEK_HEAD: [u8; 4] = [0x11, 0x4D, 0x9B, 0x74];
const ID_SEEK: [u8; 2] = [0x4D, 0xBB];
const ID_SEEK_ID: [u8; 2] = [0x53, 0xAB];
const ID_SEEK_POSITION: [u8; 2] = [0x53, 0xAC];
const ID_INFO: [u8; 4] = [0x15, 0x49, 0xA9, 0x66];
const ID_TIMESTAMP_SCALE: [u8; 3] = [0x2A, 0xD7, 0xB1];
const ID_DURATION: [u8; 2] = [0x44, 0x89];
const ID_MUXING_APP: [u8; 2] = [0x4D, 0x80];
const ID_WRITING_APP: [u8; 2] = [0x57, 0x41];
const ID_TRACKS: [u8; 4] = [0x16, 0x54, 0xAE, 0x6B];
const ID_TRACK_ENTRY: [u8; 1] = [0xAE];
const ID_TRACK_NUMBER: [u8; 1] = [0xD7];
const ID_TRACK_UID: [u8; 2] = [0x73, 0xC5];
const ID_TRACK_TYPE: [u8; 1] = [0x83];
const ID_CODEC_ID: [u8; 1] = [0x86];
const ID_CODEC_PRIVATE: [u8; 2] = [0x63, 0xA2];
const ID_VIDEO: [u8; 1] = [0xE0];
const ID_PIXEL_WIDTH: [u8; 1] = [0xB0];
const ID_PIXEL_HEIGHT: [u8; 1] = [0xBA];
const ID_AUDIO: [u8; 1] = [0xE1];
const ID_SAMPLING_FREQUENCY: [u8; 1] = [0xB5];
const ID_CHANNELS: [u8; 1] = [0x9F];
const ID_CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const ID_TIMESTAMP: [u8; 1] = [0xE7];
const ID_SIMPLE_BLOCK: [u8; 1] = [0xA3];
const ID_CUES: [u8; 4] = [0x1C, 0x53, 0xBB, 0x6B];
const ID_CUE_POINT: [u8; 1] = [0xBB];
const ID_CUE_TIME: [u8; 1] = [0xB3];
const ID_CUE_TRACK_POSITIONS: [u8; 1] = [0xB7];
const ID_CUE_TRACK: [u8; 1] = [0xF7];
const ID_CUE_CLUSTER_POSITION: [u8; 1] = [0xF1];

/// `TrackType` value for a video track (RFC 9559 §27 `TrackType`).
const TRACK_TYPE_VIDEO: u64 = 1;
/// `TrackType` value for an audio track.
const TRACK_TYPE_AUDIO: u64 = 2;
/// `SimpleBlock` keyframe flag (bit `[7]` of the flags byte, RFC 9559 §12).
const BLOCK_FLAG_KEYFRAME: u8 = 0x80;

/// `TimestampScale` this writer always emits: 1 ms per tick (RFC 9559 §27
/// default, and the value that makes a [`crate::webm_demux::WebmDemux`]-sourced
/// [`Media`] — whose IR timescale is also milliseconds — rescale losslessly).
const TIMESTAMP_SCALE_NS: u64 = 1_000_000;
/// Nanoseconds per second (track-timescale → `TimestampScale`-tick conversion).
const NS_PER_SECOND: u64 = 1_000_000_000;
/// `MuxingApp` / `WritingApp` value this writer emits.
const APP_NAME: &[u8] = b"transmux";

/// Maximum ticks (ms, given [`TIMESTAMP_SCALE_NS`]) between `Cluster`
/// boundaries when the [`Media`] has at least one video track (a new
/// `Cluster` also starts at every video keyframe, whichever comes first).
const MAX_CLUSTER_SPAN_WITH_VIDEO_TICKS: i64 = 5_000;
/// Maximum ticks between `Cluster` boundaries for an audio-only [`Media`].
const MAX_CLUSTER_SPAN_AUDIO_ONLY_TICKS: i64 = 1_000;

/// Fixed byte width `SeekPosition` entries are written at (RFC 8794 §4 allows
/// any *value* to be stored in more bytes than its minimal encoding — the
/// magnitude is unaffected by leading zero bytes): this keeps `SeekHead`'s own
/// serialized length independent of the position *values* it carries, so it
/// can be sized from a placeholder build before the real positions (which
/// depend on that very length) are known. 4 bytes bounds a `SeekPosition` to
/// 4 GiB, ample for `Info`/`Tracks` (always well under 64 KiB in practice).
const SEEK_POSITION_FIXED_WIDTH: usize = 4;

// ---------------------------------------------------------------------------
// Matroska CodecIDs (RFC 9559 §27 codec-mapping registry) — mirrors the set
// `crate::webm_demux::WebmDemux` maps in the opposite direction, plus the
// ISOBMFF-family video/audio codecs a DVR recording carries.
// ---------------------------------------------------------------------------

const CODEC_V_AVC: &[u8] = b"V_MPEG4/ISO/AVC";
const CODEC_V_HEVC: &[u8] = b"V_MPEGH/ISO/HEVC";
const CODEC_V_VP9: &[u8] = b"V_VP9";
const CODEC_V_VP8: &[u8] = b"V_VP8";
const CODEC_V_AV1: &[u8] = b"V_AV1";
const CODEC_A_AAC: &[u8] = b"A_AAC";
const CODEC_A_OPUS: &[u8] = b"A_OPUS";
const CODEC_A_VORBIS: &[u8] = b"A_VORBIS";
const CODEC_A_AC3: &[u8] = b"A_AC3";
const CODEC_A_EAC3: &[u8] = b"A_EAC3";

/// `OpusHead` identification-header magic (RFC 7845 §5.1).
const OPUS_HEAD_MAGIC: &[u8; 8] = b"OpusHead";

// ---------------------------------------------------------------------------
// EBML element write helpers (RFC 8794 §4)
// ---------------------------------------------------------------------------

/// Encode an EBML element **size** VINT (RFC 8794 §4): the minimal width (1-8
/// bytes) whose leading `1` marker bit plus the remaining data bits can
/// represent `value`. The same encoding also serves an element **ID**'s
/// *referenced* value where a VINT-with-marker is required (e.g. a
/// `SimpleBlock`'s track number, RFC 9559 §12) — width selection is identical.
///
/// Values at or above `2^56` would need a 9+-byte encoding this function does
/// not produce (EBML's own DocType/element-ID width caps make this
/// unreachable for anything this module writes — element sizes, byte offsets,
/// and track numbers all stay far below that bound in practice).
fn encode_vint_size(value: u64) -> Vec<u8> {
    let width: usize = if value <= 0x7E {
        1
    } else if value <= 0x3FFE {
        2
    } else if value <= 0x1F_FFFE {
        3
    } else if value <= 0x0FFF_FFFE {
        4
    } else if value <= 0x07_FFFF_FFFE {
        5
    } else {
        8
    };
    let marker = 1u64 << (width * 7);
    let encoded = marker | value;
    encoded.to_be_bytes()[8 - width..].to_vec()
}

/// Write one EBML element: `id` bytes, then the VINT size of `body`, then
/// `body` verbatim.
fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
    let size = encode_vint_size(body.len() as u64);
    let mut out = Vec::with_capacity(id.len() + size.len() + body.len());
    out.extend_from_slice(id);
    out.extend_from_slice(&size);
    out.extend_from_slice(body);
    out
}

/// Write a master element: the concatenation of `children`'s already-encoded
/// bytes, wrapped in `id`/size.
fn master(id: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    for child in children {
        body.extend_from_slice(child);
    }
    element(id, &body)
}

/// The minimal big-endian byte encoding of a uint element body (RFC 8794 §4:
/// a uint element's value is however many bytes are present, big-endian; `0`
/// still needs one byte).
fn uint_bytes(value: u64) -> Vec<u8> {
    if value == 0 {
        return vec![0u8];
    }
    let full = value.to_be_bytes();
    let first_nonzero = full.iter().position(|&b| b != 0).unwrap_or(7);
    full[first_nonzero..].to_vec()
}

/// Write a uint element at its minimal byte width.
fn uint_elem(id: &[u8], value: u64) -> Vec<u8> {
    element(id, &uint_bytes(value))
}

/// Write a uint element at a caller-fixed byte width (padded with leading
/// zero bytes) — see [`SEEK_POSITION_FIXED_WIDTH`] for why this exists.
fn fixed_uint_elem(id: &[u8], value: u64, width: usize) -> Vec<u8> {
    let full = value.to_be_bytes();
    element(id, &full[8 - width..])
}

/// Write a string/UTF-8 element (verbatim ASCII/UTF-8 bytes, no terminator).
fn string_elem(id: &[u8], s: &[u8]) -> Vec<u8> {
    element(id, s)
}

/// Write a binary element (verbatim opaque bytes — `CodecPrivate`, …).
fn binary_elem(id: &[u8], data: &[u8]) -> Vec<u8> {
    element(id, data)
}

/// Write a float element (8-byte IEEE 754 big-endian — this writer always
/// uses the 64-bit float width for `Duration`/`SamplingFrequency`).
fn float64_elem(id: &[u8], value: f64) -> Vec<u8> {
    element(id, &value.to_be_bytes())
}

// ---------------------------------------------------------------------------
// EBML Header
// ---------------------------------------------------------------------------

fn build_ebml_header() -> Vec<u8> {
    master(
        &ID_EBML,
        &[
            uint_elem(&ID_EBML_VERSION, EBML_VERSION_VALUE),
            uint_elem(&ID_EBML_READ_VERSION, EBML_READ_VERSION_VALUE),
            uint_elem(&ID_EBML_MAX_ID_LENGTH, EBML_MAX_ID_LENGTH_VALUE),
            uint_elem(&ID_EBML_MAX_SIZE_LENGTH, EBML_MAX_SIZE_LENGTH_VALUE),
            string_elem(&ID_DOC_TYPE, DOC_TYPE),
            uint_elem(&ID_DOC_TYPE_VERSION, DOC_TYPE_VERSION_VALUE),
            uint_elem(&ID_DOC_TYPE_READ_VERSION, DOC_TYPE_READ_VERSION_VALUE),
        ],
    )
}

// ---------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------

fn build_info(duration_ticks: f64) -> Vec<u8> {
    master(
        &ID_INFO,
        &[
            uint_elem(&ID_TIMESTAMP_SCALE, TIMESTAMP_SCALE_NS),
            float64_elem(&ID_DURATION, duration_ticks),
            string_elem(&ID_MUXING_APP, APP_NAME),
            string_elem(&ID_WRITING_APP, APP_NAME),
        ],
    )
}

// ---------------------------------------------------------------------------
// SeekHead
// ---------------------------------------------------------------------------

/// One `Seek` entry: the target element's ID, and its byte offset relative to
/// the `Segment` payload start, written at [`SEEK_POSITION_FIXED_WIDTH`].
fn seek_entry(target_id: &[u8], position: u32) -> Vec<u8> {
    master(
        &ID_SEEK,
        &[
            binary_elem(&ID_SEEK_ID, target_id),
            fixed_uint_elem(
                &ID_SEEK_POSITION,
                position as u64,
                SEEK_POSITION_FIXED_WIDTH,
            ),
        ],
    )
}

/// Build `SeekHead`, indexing `Info` and `Tracks` at the given byte offsets
/// (relative to the `Segment` payload start).
fn build_seek_head(pos_info: u32, pos_tracks: u32) -> Vec<u8> {
    master(
        &ID_SEEK_HEAD,
        &[
            seek_entry(&ID_INFO, pos_info),
            seek_entry(&ID_TRACKS, pos_tracks),
        ],
    )
}

// ---------------------------------------------------------------------------
// Tracks / TrackEntry
// ---------------------------------------------------------------------------

fn video_elem(width: u16, height: u16) -> Vec<u8> {
    master(
        &ID_VIDEO,
        &[
            uint_elem(&ID_PIXEL_WIDTH, width as u64),
            uint_elem(&ID_PIXEL_HEIGHT, height as u64),
        ],
    )
}

fn audio_elem(channels: u16, sample_rate: u32) -> Vec<u8> {
    master(
        &ID_AUDIO,
        &[
            uint_elem(&ID_CHANNELS, channels as u64),
            float64_elem(&ID_SAMPLING_FREQUENCY, sample_rate as f64),
        ],
    )
}

/// Rebuild the raw `OpusHead` identification header (RFC 7845 §5.1) from a
/// decoded [`OpusSpecificBox`] — the exact inverse of
/// `webm_demux::opus_config`'s parse.
fn build_opus_head(dops: &OpusSpecificBox) -> Vec<u8> {
    let mut out = Vec::with_capacity(19);
    out.extend_from_slice(OPUS_HEAD_MAGIC);
    out.push(dops.version);
    out.push(dops.output_channel_count);
    out.extend_from_slice(&dops.pre_skip.to_le_bytes());
    out.extend_from_slice(&dops.input_sample_rate.to_le_bytes());
    out.extend_from_slice(&dops.output_gain.to_le_bytes());
    out.push(dops.channel_mapping_family);
    if let Some(map) = &dops.channel_mapping {
        out.push(map.stream_count);
        out.push(map.coupled_count);
        out.extend_from_slice(&map.channel_mapping);
    }
    out
}

/// A human-readable name for a [`CodecConfig`] variant, for
/// [`Error::UnsupportedMkvCodec`]'s message.
fn codec_debug_name(config: &CodecConfig) -> &'static str {
    match config {
        CodecConfig::Avc { .. } => "AVC",
        CodecConfig::Hevc { .. } => "HEVC",
        CodecConfig::Vvc { .. } => "VVC",
        CodecConfig::Aac { .. } => "AAC",
        CodecConfig::Ac3 { .. } => "AC-3",
        CodecConfig::Eac3 { .. } => "E-AC-3",
        CodecConfig::Av1 { .. } => "AV1",
        CodecConfig::Vp9 { .. } => "VP9",
        CodecConfig::Vp8 { .. } => "VP8",
        CodecConfig::Opus { .. } => "Opus",
        CodecConfig::Flac { .. } => "FLAC",
        CodecConfig::Ac4 { .. } => "AC-4",
        CodecConfig::MpegH { .. } => "MPEG-H 3D Audio",
        CodecConfig::Mpeg2Video { .. } => "MPEG-2 Video",
        CodecConfig::MpegAudio { .. } => "MPEG-1/2 Audio",
        CodecConfig::Dts { .. } => "DTS",
        CodecConfig::Vorbis { .. } => "Vorbis",
        CodecConfig::Subtitle { .. } => "Subtitle",
        CodecConfig::Data { .. } => "Data",
    }
}

/// Build one `TrackEntry`, returning its bytes plus whether it is a video
/// track. `track_number` is the 1-based Matroska track number (this crate
/// assigns them sequentially in [`Media::tracks`] order); `track_id` supplies
/// `TrackUID`.
fn build_track_entry(
    track_number: u64,
    track_id: u32,
    config: &CodecConfig,
) -> Result<(Vec<u8>, bool)> {
    let is_video = config.is_video();
    let track_type = if is_video {
        TRACK_TYPE_VIDEO
    } else {
        TRACK_TYPE_AUDIO
    };

    let (codec_id, codec_private, sub_elem): (&'static [u8], Option<Vec<u8>>, Vec<u8>) =
        match config {
            CodecConfig::Avc {
                config,
                width,
                height,
            } => (
                CODEC_V_AVC,
                Some(config.config.to_bytes()),
                video_elem(*width, *height),
            ),
            CodecConfig::Hevc {
                config,
                width,
                height,
            } => (
                CODEC_V_HEVC,
                Some(config.config.to_bytes()),
                video_elem(*width, *height),
            ),
            CodecConfig::Vp9 { width, height, .. } => {
                (CODEC_V_VP9, None, video_elem(*width, *height))
            }
            CodecConfig::Vp8 { width, height } => (CODEC_V_VP8, None, video_elem(*width, *height)),
            CodecConfig::Av1 {
                config,
                width,
                height,
            } => (
                CODEC_V_AV1,
                Some(config.to_bytes()),
                video_elem(*width, *height),
            ),
            CodecConfig::Aac {
                esds,
                channel_count,
                sample_rate,
                ..
            } => {
                let asc = esds
                    .es_descriptor
                    .decoder_config
                    .as_ref()
                    .and_then(|dc| dc.decoder_specific_info.as_ref())
                    .map(|dsi| dsi.data.clone())
                    .ok_or(Error::InvalidInput(
                        "AAC esds has no DecoderSpecificInfo (AudioSpecificConfig) to carry as \
                     Matroska CodecPrivate",
                    ))?;
                (
                    CODEC_A_AAC,
                    Some(asc),
                    audio_elem(*channel_count, *sample_rate),
                )
            }
            CodecConfig::Opus {
                config,
                channel_count,
                sample_rate,
                ..
            } => (
                CODEC_A_OPUS,
                Some(build_opus_head(config)),
                audio_elem(*channel_count, *sample_rate),
            ),
            CodecConfig::Vorbis {
                codec_private,
                channels,
                sample_rate,
            } => (
                CODEC_A_VORBIS,
                Some(codec_private.clone()),
                audio_elem(*channels, *sample_rate),
            ),
            CodecConfig::Ac3 {
                channel_count,
                sample_rate,
                ..
            } => (CODEC_A_AC3, None, audio_elem(*channel_count, *sample_rate)),
            CodecConfig::Eac3 {
                channel_count,
                sample_rate,
                ..
            } => (CODEC_A_EAC3, None, audio_elem(*channel_count, *sample_rate)),
            other => {
                return Err(Error::UnsupportedMkvCodec {
                    codec: codec_debug_name(other),
                });
            }
        };

    let mut children = vec![
        uint_elem(&ID_TRACK_NUMBER, track_number),
        uint_elem(&ID_TRACK_UID, track_id as u64),
        uint_elem(&ID_TRACK_TYPE, track_type),
        string_elem(&ID_CODEC_ID, codec_id),
        sub_elem,
    ];
    if let Some(priv_bytes) = codec_private {
        children.push(binary_elem(&ID_CODEC_PRIVATE, &priv_bytes));
    }
    Ok((master(&ID_TRACK_ENTRY, &children), is_video))
}

// ---------------------------------------------------------------------------
// Cluster / SimpleBlock / Cues
// ---------------------------------------------------------------------------

/// One sample, resolved to its global Matroska (`TimestampScale`-tick)
/// presentation time, ready to be placed into a `Cluster`.
#[derive(Debug, Clone, Copy)]
struct BlockEvent {
    /// Index into [`Media::tracks`].
    track_index: usize,
    /// 1-based Matroska `TrackNumber`.
    track_number: u64,
    /// Index into the track's `samples`.
    sample_index: usize,
    /// Absolute presentation time, in `TimestampScale` ticks.
    pts_ticks: i64,
    /// Whether this sample is a sync sample (random-access point).
    is_keyframe: bool,
    /// Whether this event's track is video.
    is_video: bool,
}

/// Convert a tick value in `track_timescale` units to `TimestampScale` ticks
/// (the inverse of `webm_demux::parse_block`'s `raw_ticks * scale_ns /
/// ns_per_ir_tick`): `ticks / track_timescale` seconds, expressed in
/// [`TIMESTAMP_SCALE_NS`]-sized ticks.
fn to_matroska_ticks(ticks: i64, track_timescale: u32, timestamp_scale_ns: u64) -> i64 {
    if track_timescale == 0 {
        return 0;
    }
    let ns = (ticks as i128) * (NS_PER_SECOND as i128) / (track_timescale as i128);
    (ns / timestamp_scale_ns as i128) as i64
}

/// Write one `SimpleBlock` (RFC 9559 §12): track-number VINT, signed int16
/// relative timestamp, flags byte (keyframe bit only — no lacing), frame data.
fn simple_block(track_number: u64, rel_ts: i16, is_keyframe: bool, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(&encode_vint_size(track_number));
    body.extend_from_slice(&rel_ts.to_be_bytes());
    body.push(if is_keyframe { BLOCK_FLAG_KEYFRAME } else { 0 });
    body.extend_from_slice(data);
    element(&ID_SIMPLE_BLOCK, &body)
}

/// Group a time-interleaved (decode-order-preserving per track — see
/// [`MkvMux`]'s `package` impl) [`BlockEvent`] stream into `Cluster`s: a new
/// `Cluster` starts at every video keyframe (when any track is video), and in
/// any case at least every `max_span` ticks. See the module docs.
fn build_clusters(events: &[BlockEvent], has_video: bool) -> Vec<Vec<BlockEvent>> {
    let max_span = if has_video {
        MAX_CLUSTER_SPAN_WITH_VIDEO_TICKS
    } else {
        MAX_CLUSTER_SPAN_AUDIO_ONLY_TICKS
    };
    let mut clusters: Vec<Vec<BlockEvent>> = Vec::new();
    let mut current: Vec<BlockEvent> = Vec::new();
    let mut cluster_start: i64 = 0;
    for &ev in events {
        let start_new = current.is_empty()
            || (has_video && ev.is_video && ev.is_keyframe)
            || (ev.pts_ticks - cluster_start >= max_span);
        if start_new && !current.is_empty() {
            clusters.push(core::mem::take(&mut current));
        }
        if current.is_empty() {
            cluster_start = ev.pts_ticks;
        }
        current.push(ev);
    }
    if !current.is_empty() {
        clusters.push(current);
    }
    clusters
}

/// Serialize one `Cluster`: its base `Timestamp` (the first event's presentation
/// time) then a `SimpleBlock` per event, in order.
fn build_cluster(events: &[BlockEvent], tracks: &[Track]) -> Vec<u8> {
    let cluster_start = events.first().map(|e| e.pts_ticks).unwrap_or(0);
    let mut children = vec![uint_elem(&ID_TIMESTAMP, cluster_start.max(0) as u64)];
    for ev in events {
        let rel_i64 = ev.pts_ticks - cluster_start;
        debug_assert!(
            (i16::MIN as i64..=i16::MAX as i64).contains(&rel_i64),
            "cluster relative timestamp {rel_i64} exceeds i16 range \
             (build_clusters's max-span invariant was violated)"
        );
        let rel = rel_i64 as i16;
        let data = &tracks[ev.track_index].samples[ev.sample_index].data;
        children.push(simple_block(ev.track_number, rel, ev.is_keyframe, data));
    }
    master(&ID_CLUSTER, &children)
}

/// Build `Cues`: one `CuePoint` per **video** keyframe (never audio — every
/// audio sample in this IR is a sync sample, so indexing them would defeat
/// the point of an index). Returns an empty `Vec` (nothing written) when
/// there is no video keyframe to index.
fn build_cues(clusters: &[Vec<BlockEvent>], cluster_offsets: &[u32]) -> Vec<u8> {
    let mut cue_points: Vec<Vec<u8>> = Vec::new();
    for (cluster, &offset) in clusters.iter().zip(cluster_offsets) {
        for ev in cluster {
            if ev.is_video && ev.is_keyframe {
                let track_positions = master(
                    &ID_CUE_TRACK_POSITIONS,
                    &[
                        uint_elem(&ID_CUE_TRACK, ev.track_number),
                        uint_elem(&ID_CUE_CLUSTER_POSITION, offset as u64),
                    ],
                );
                cue_points.push(master(
                    &ID_CUE_POINT,
                    &[
                        uint_elem(&ID_CUE_TIME, ev.pts_ticks.max(0) as u64),
                        track_positions,
                    ],
                ));
            }
        }
    }
    if cue_points.is_empty() {
        return Vec::new();
    }
    master(&ID_CUES, &cue_points)
}

// ---------------------------------------------------------------------------
// MkvMux
// ---------------------------------------------------------------------------

/// Package a [`Media`] into a Matroska (`.mkv`) file.
///
/// Implements [`broadcast_common::Package`] with `Output = Vec<u8>`: the
/// whole file (EBML header + `Segment`) is returned as one byte vector. See
/// the module docs for scope, cluster strategy, and the `SeekHead`/`Cues`
/// layout.
#[derive(Debug, Clone, Copy, Default)]
pub struct MkvMux {
    _private: (),
}

impl MkvMux {
    /// Create a new muxer.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Package for MkvMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }

        // Resolve every TrackEntry up front (fails fast, naming the track, for
        // any codec this module has no Matroska CodecID mapping for).
        let mut track_entries: Vec<Vec<u8>> = Vec::with_capacity(media.tracks.len());
        let mut is_video_track: Vec<bool> = Vec::with_capacity(media.tracks.len());
        for (i, track) in media.tracks.iter().enumerate() {
            let track_number = (i + 1) as u64;
            let (entry, is_video) =
                build_track_entry(track_number, track.spec.track_id, &track.spec.config)?;
            track_entries.push(entry);
            is_video_track.push(is_video);
        }
        let has_video = is_video_track.iter().any(|&v| v);

        // Every track's own samples are in **decode** order (`Track::samples`'
        // documented invariant) — for AVC/HEVC with B-frame reordering, decode
        // order is not presentation-time order (a P-frame decodes before, but
        // may present after, the B-frames that reference it), so a track's own
        // `pts_ticks` sequence is not necessarily monotonic. Interleaving
        // tracks by a global sort on `pts_ticks` would therefore reorder a
        // track's blocks relative to each other and desync decode from
        // reference — instead, a k-way merge walks each track's cursor forward
        // one sample at a time (never reordering *within* a track) and always
        // takes whichever track's *next* sample has the smallest presentation
        // time, which only decides *when* to interleave the tracks.
        let per_track_ticks: Vec<Vec<i64>> = media
            .tracks
            .iter()
            .map(|track| {
                let timescale = track.timescale();
                track
                    .samples
                    .iter()
                    .map(|s| {
                        to_matroska_ticks(
                            s.pts.or(s.dts).unwrap_or(0),
                            timescale,
                            TIMESTAMP_SCALE_NS,
                        )
                    })
                    .collect()
            })
            .collect();

        let mut duration_ticks: i64 = 0;
        for track in &media.tracks {
            let timescale = track.timescale();
            for sample in &track.samples {
                let pts = sample.pts.or(sample.dts).unwrap_or(0);
                let end_ticks = to_matroska_ticks(
                    pts + sample.duration.unwrap_or(0) as i64,
                    timescale,
                    TIMESTAMP_SCALE_NS,
                );
                if end_ticks > duration_ticks {
                    duration_ticks = end_ticks;
                }
            }
        }

        let mut cursors = vec![0usize; media.tracks.len()];
        let mut events: Vec<BlockEvent> = Vec::new();
        loop {
            let mut best: Option<(usize, i64)> = None;
            for (i, track) in media.tracks.iter().enumerate() {
                if cursors[i] < track.samples.len() {
                    let pts_ticks = per_track_ticks[i][cursors[i]];
                    if best.is_none_or(|(_, best_ticks)| pts_ticks < best_ticks) {
                        best = Some((i, pts_ticks));
                    }
                }
            }
            let Some((i, pts_ticks)) = best else { break };
            let si = cursors[i];
            cursors[i] += 1;
            events.push(BlockEvent {
                track_index: i,
                track_number: (i + 1) as u64,
                sample_index: si,
                pts_ticks,
                is_keyframe: media.tracks[i].samples[si].flags.is_sync,
                is_video: is_video_track[i],
            });
        }

        let info_bytes = build_info(duration_ticks as f64);
        let tracks_bytes = master(&ID_TRACKS, &track_entries);

        // SeekHead: sized from a placeholder build (see SEEK_POSITION_FIXED_WIDTH),
        // then rebuilt with the real Info/Tracks offsets at the same size.
        let seekhead_placeholder = build_seek_head(0, 0);
        let pos_info = seekhead_placeholder.len() as u32;
        let pos_tracks = pos_info + info_bytes.len() as u32;
        let seekhead_bytes = build_seek_head(pos_info, pos_tracks);
        debug_assert_eq!(
            seekhead_bytes.len(),
            seekhead_placeholder.len(),
            "SeekHead's serialized size must be independent of its SeekPosition values"
        );

        let clusters = build_clusters(&events, has_video);
        let mut cluster_bytes: Vec<Vec<u8>> = Vec::with_capacity(clusters.len());
        let mut cluster_offsets: Vec<u32> = Vec::with_capacity(clusters.len());
        let mut running_offset = pos_tracks + tracks_bytes.len() as u32;
        for cluster in &clusters {
            cluster_offsets.push(running_offset);
            let bytes = build_cluster(cluster, &media.tracks);
            running_offset += bytes.len() as u32;
            cluster_bytes.push(bytes);
        }

        let cues_bytes = build_cues(&clusters, &cluster_offsets);

        let mut segment_payload = Vec::new();
        segment_payload.extend_from_slice(&seekhead_bytes);
        segment_payload.extend_from_slice(&info_bytes);
        segment_payload.extend_from_slice(&tracks_bytes);
        for c in &cluster_bytes {
            segment_payload.extend_from_slice(c);
        }
        segment_payload.extend_from_slice(&cues_bytes);

        let mut out = build_ebml_header();
        out.extend_from_slice(&element(&ID_SEGMENT, &segment_payload));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_size_widths() {
        assert_eq!(encode_vint_size(0), vec![0x80]);
        assert_eq!(encode_vint_size(1), vec![0x81]);
        assert_eq!(encode_vint_size(0x7E), vec![0xFE]);
        assert_eq!(encode_vint_size(0x7F), vec![0x40, 0x7F]);
        assert_eq!(encode_vint_size(2), vec![0x82]);
    }

    #[test]
    fn uint_bytes_widths() {
        assert_eq!(uint_bytes(0), vec![0u8]);
        assert_eq!(uint_bytes(1), vec![1u8]);
        assert_eq!(uint_bytes(1_000_000), vec![0x0F, 0x42, 0x40]);
    }

    #[test]
    fn opus_head_round_trips_via_field_reconstruction() {
        let dops = OpusSpecificBox {
            version: 1,
            output_channel_count: 1,
            pre_skip: 312,
            input_sample_rate: 48_000,
            output_gain: 0,
            channel_mapping_family: 0,
            channel_mapping: None,
        };
        let head = build_opus_head(&dops);
        assert_eq!(&head[0..8], OPUS_HEAD_MAGIC);
        assert_eq!(head[8], 1); // version
        assert_eq!(head[9], 1); // channels
        assert_eq!(u16::from_le_bytes([head[10], head[11]]), 312);
        assert_eq!(
            u32::from_le_bytes([head[12], head[13], head[14], head[15]]),
            48_000
        );
    }

    #[test]
    fn empty_media_rejected() {
        let media = Media::new(Vec::new(), 1000);
        let mut mux = MkvMux::new();
        assert!(mux.package(&media).is_err());
    }
}
