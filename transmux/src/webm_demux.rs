//! WebM / Matroska (EBML) demuxer — WebM byte stream → the [`Media`] IR.
//!
//! Walks the EBML element tree of a WebM file (EBML header → Segment → Info /
//! Tracks / Cluster*) and produces a [`Media`] of elementary [`Track`]s of coded
//! [`Sample`]s, implementing [`broadcast_common::Unpackage`] so `WebM → IR →
//! {any}` composes with the rest of the crate's packagers.
//!
//! # Spec citations
//!
//! - **EBML framing** (VINT element-ID / element-size): RFC 8794 §4.
//! - **Matroska element IDs / semantics** (Segment, Info, Tracks, Cluster,
//!   (Simple)Block): RFC 9559 §12 / §27.
//! - Element-ID table, (Simple)Block layout and the CodecID → [`CodecConfig`]
//!   mapping are transcribed in `transmux/docs/webm/ebml-matroska.md`.
//! - **VP8** key-frame header (dimensions): RFC 6386 §9.1 / §19.1.
//! - **Vorbis** `CodecPrivate` (Xiph-laced 3 headers) + Identification header:
//!   the Vorbis I specification §4.2.2. Both are transcribed in
//!   `transmux/docs/codec/vp8-vorbis-webm.md`.
//!
//! # Scope
//!
//! Only the elements needed to demux WebM/Matroska video + audio are decoded;
//! `SeekHead`, `Cues`, `Tags` and any other master element are skipped by size.
//! The mapped CodecIDs are `V_VP9` (→ [`CodecConfig::Vp9`]), `V_VP8` (→
//! [`CodecConfig::Vp8`]), `V_MPEG4/ISO/AVC` (→ [`CodecConfig::Avc`]),
//! `V_MPEGH/ISO/HEVC` (→ [`CodecConfig::Hevc`]), `A_OPUS` (→
//! [`CodecConfig::Opus`]), `A_VORBIS` (→ [`CodecConfig::Vorbis`]) and `A_AAC`
//! (→ [`CodecConfig::Aac`]); every other CodecID is skipped (never fatal).
//! (`crate::mkv_mux::MkvMux`, the inverse packager, mirrors this exact set.)
//! Laced blocks are unlaced into their constituent frames (all three encodings
//! of RFC 9559 §12 — Xiph, EBML and fixed-size). Lacing is the norm for
//! mkvmerge-muxed audio, so treating it as fatal rejected every ordinary
//! mkvmerge Matroska file outright. Each frame becomes its own sample, timed
//! from the block's timestamp at the track's frame cadence (`DefaultDuration`
//! when the TrackEntry declares one, else the interval to the next block); a
//! block carries at most the format's 256 frames, and a zero-length lace is
//! rejected.
//!
//! # Timescale
//!
//! Matroska carries **presentation** timestamps. Cluster + block timestamps are
//! in `TimestampScale`-ns ticks (default 1_000_000 ns = 1 ms). The IR uses a
//! **millisecond** timescale ([`IR_TIMESCALE`] = 1000): a presentation time of
//! `(cluster_ts + rel_ts)` ticks × `TimestampScale` ns is converted to
//! milliseconds. VP9/Opus have no B-frame reorder, so DTS == PTS and every
//! sample's `composition_offset` is 0. Per-sample `duration` is the delta to the
//! next block's presentation time (the final block reuses the previous delta, or
//! the track's `DefaultDuration` when only one block is present).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Parse, Unpackage};

use crate::annexb::{NAL_LENGTH_SIZE_MINUS_ONE, normalise_nal_length_size};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use crate::media::{Media, Track};
use crate::opus::OpusSpecificBox;
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::rtp_sdp::aac_config_from_asc_bytes;
use crate::vp9::Vp9ConfigurationBox;

// --- EBML / Matroska element IDs (RFC 9559 §27; stored with marker bits) -----
/// `EBML` header master element.
const EBML_HEADER: u32 = 0x1A45_DFA3;
/// `Segment` top-level master element.
const SEGMENT: u32 = 0x1853_8067;
/// `Info` master element (Segment child).
const INFO: u32 = 0x1549_A966;
/// `TimestampScale` (ns per tick; Info child).
const TIMESTAMP_SCALE: u32 = 0x2A_D7_B1;
/// `Tracks` master element (Segment child).
const TRACKS: u32 = 0x1654_AE6B;
/// `TrackEntry` master element (Tracks child).
const TRACK_ENTRY: u32 = 0xAE;
/// `TrackNumber` (TrackEntry child).
const TRACK_NUMBER: u32 = 0xD7;
/// `TrackType` (TrackEntry child; 1 = video, 2 = audio).
const TRACK_TYPE: u32 = 0x83;
/// `CodecID` (TrackEntry child).
const CODEC_ID: u32 = 0x86;
/// `CodecPrivate` (TrackEntry child; codec setup — e.g. OpusHead).
const CODEC_PRIVATE: u32 = 0x63A2;
/// `DefaultDuration` (TrackEntry child; ns per frame).
const DEFAULT_DURATION: u32 = 0x23_E3_83;
/// `Video` master element (TrackEntry child).
const VIDEO: u32 = 0xE0;
/// `PixelWidth` (Video child).
const PIXEL_WIDTH: u32 = 0xB0;
/// `PixelHeight` (Video child).
const PIXEL_HEIGHT: u32 = 0xBA;
/// `Audio` master element (TrackEntry child).
const AUDIO: u32 = 0xE1;
/// `SamplingFrequency` (Audio child; Hz float).
const SAMPLING_FREQUENCY: u32 = 0xB5;
/// `Channels` (Audio child).
const CHANNELS: u32 = 0x9F;
/// `Cluster` master element (Segment child).
const CLUSTER: u32 = 0x1F43_B675;
/// `Timestamp` (Cluster base time, in TimestampScale ticks).
const CLUSTER_TIMESTAMP: u32 = 0xE7;
/// `SimpleBlock` (Cluster child; block layout with a keyframe flag).
const SIMPLE_BLOCK: u32 = 0xA3;
/// `BlockGroup` master element (Cluster child).
const BLOCK_GROUP: u32 = 0xA0;
/// `Block` (BlockGroup child; block layout, no keyframe flag).
const BLOCK: u32 = 0xA1;
/// `SeekHead` master element (Segment child; not decoded — see
/// `SEGMENT_LEVEL_IDS`).
const SEEK_HEAD: u32 = 0x114D_9B74;
/// `Cues` master element (Segment child; not decoded).
const CUES: u32 = 0x1C53_BB6B;
/// `Tags` master element (Segment child; not decoded).
const TAGS: u32 = 0x1254_C367;
/// `Chapters` master element (Segment child; not decoded).
const CHAPTERS: u32 = 0x1043_A770;
/// `Attachments` master element (Segment child; not decoded).
const ATTACHMENTS: u32 = 0x1941_A469;
/// Every element ID this crate recognizes as a direct `Segment` child,
/// decoded or not. Used only to terminate an **unknown-size** Segment child
/// early (C10, #1011): RFC 8794 §6.2 / the Matroska DTD end an unknown-size
/// element at the first element that is not its own descendant, so any of
/// these appearing while walking one is itself a fresh Segment-level sibling,
/// never a valid child of the element still open — most commonly the next
/// `Cluster`, since live encoders (browser `MediaRecorder`, `ffmpeg -f webm
/// -live 1`, OBS) write every Cluster unknown-size.
const SEGMENT_LEVEL_IDS: &[u32] = &[
    INFO,
    TRACKS,
    CLUSTER,
    SEEK_HEAD,
    CUES,
    TAGS,
    CHAPTERS,
    ATTACHMENTS,
];

/// `ReferenceBlock` (BlockGroup child; present ⇒ the Block references another
/// block ⇒ not a random-access point). Its absence marks the Block a keyframe.
const REFERENCE_BLOCK: u32 = 0xFB;

// --- Matroska CodecIDs (RFC 9559 codec-mapping registry) ---------------------
/// VP9 video CodecID.
const CODEC_V_VP9: &[u8] = b"V_VP9";
/// VP8 video CodecID.
const CODEC_V_VP8: &[u8] = b"V_VP8";
/// H.264/AVC video CodecID; `CodecPrivate` is the raw `AVCDecoderConfigurationRecord`
/// (ISO/IEC 14496-15 §5.3.3 — the `avcC` box body, with no box header).
const CODEC_V_AVC: &[u8] = b"V_MPEG4/ISO/AVC";
/// H.265/HEVC video CodecID; `CodecPrivate` is the raw `HEVCDecoderConfigurationRecord`
/// (ISO/IEC 14496-15 §8.3.3.1 — the `hvcC` box body, with no box header).
const CODEC_V_HEVC: &[u8] = b"V_MPEGH/ISO/HEVC";
/// Opus audio CodecID.
const CODEC_A_OPUS: &[u8] = b"A_OPUS";
/// Vorbis audio CodecID.
const CODEC_A_VORBIS: &[u8] = b"A_VORBIS";
/// AAC audio CodecID; `CodecPrivate` is the raw `AudioSpecificConfig`
/// (ISO/IEC 14496-3 §1.6.2.1 — the same bytes an ISOBMFF `esds`'s
/// `DecoderSpecificInfo` carries).
const CODEC_A_AAC: &[u8] = b"A_AAC";

// --- TrackType values (RFC 9559 §27 `TrackType`) -----------------------------
/// `TrackType` value for a video track.
const TRACK_TYPE_VIDEO: u64 = 1;
/// `TrackType` value for an audio track.
const TRACK_TYPE_AUDIO: u64 = 2;

// --- (Simple)Block flag bits (RFC 9559 §12) ----------------------------------
/// SimpleBlock keyframe flag (bit `[7]` of the flags byte).
const BLOCK_FLAG_KEYFRAME: u8 = 0x80;
/// Lacing bits mask (bits `[2:1]` of the flags byte); non-zero = laced.
const BLOCK_FLAG_LACING_MASK: u8 = 0x06;
/// Lacing mode (bits `[2:1]`): `0b01` = Xiph lacing.
const BLOCK_FLAG_LACING_XIPH: u8 = 0x02;
/// Lacing mode (bits `[2:1]`): `0b11` = EBML lacing.
const BLOCK_FLAG_LACING_EBML: u8 = 0x06;
/// Lacing mode (bits `[2:1]`): `0b10` = fixed-size lacing. (RFC 9559 §12: the
/// three encodings are chosen by these two bits, with `0b00` meaning no lacing.)
const BLOCK_FLAG_LACING_FIXED: u8 = 0x04;
/// Bytes per Xiph-lace size run: each byte holds 0-255 and a value of 255
/// continues the run (RFC 9559 §12, Xiph lacing).
const XIPH_LACE_CONTINUATION: u8 = 0xFF;
/// Maximum frames one laced block may carry: the count byte is
/// `FrameCount - 1` (RFC 9559 §12 "Lacing"), so the format's own ceiling is
/// `0xFF + 1` = 256. Enforced so a hostile count cannot drive per-block work
/// past the format's own bound.
const MAX_LACED_FRAMES: usize = 256;
/// Bias applied to an EBML-lacing size delta: the value is
/// `value - (2^(7*len - 1) - 1)`, i.e. the stored VINT minus this bias
/// (RFC 8794 §4 "VINT" / RFC 9559 §12 "EBML lacing", which is the same signed
/// VINT encoding). Written as an expression over the VINT's byte length so the
/// `1..=8`-byte range it is derived from is visible at the call site.
const fn ebml_lace_delta_bias(vint_len: usize) -> i64 {
    (1i64 << (7 * vint_len - 1)) - 1
}

// --- Defaults ----------------------------------------------------------------
/// Default `TimestampScale` when the `Info` element omits it (RFC 9559 §27): 1 ms.
const DEFAULT_TIMESTAMP_SCALE_NS: u64 = 1_000_000;
/// The IR timescale (ticks per second) this demuxer emits: milliseconds.
pub const IR_TIMESCALE: u32 = 1000;
/// Nanoseconds per second (TimestampScale → IR-tick conversion).
const NS_PER_SECOND: u64 = 1_000_000_000;
/// OpusHead identification-header magic (RFC 7845 §5.1).
const OPUS_HEAD_MAGIC: &[u8; 8] = b"OpusHead";
/// Minimum OpusHead length: magic(8) + version(1) + channels(1) + pre-skip(2) +
/// input-rate(4) + output-gain(2) + mapping-family(1) = 19 bytes.
const OPUS_HEAD_MIN_LEN: usize = 19;
/// Opus playback rate is always 48 kHz (RFC 7845 §5.1 / Opus-in-ISOBMFF).
const OPUS_OUTPUT_SAMPLE_RATE: u32 = 48_000;
/// Audio sample size in bits carried in the sample entry (convention: 16).
const AUDIO_SAMPLE_SIZE: u16 = 16;

// --- VP8 key-frame header (RFC 6386 §9.1 / §19.1) ----------------------------
/// VP8 uncompressed frame tag length (bytes): a 24-bit little-endian field.
const VP8_FRAME_TAG_LEN: usize = 3;
/// VP8 key-frame start code that follows the tag (RFC 6386 §9.1): `0x9D 01 2A`.
const VP8_START_CODE: [u8; 3] = [0x9D, 0x01, 0x2A];
/// `key_frame` bit is bit `[0]` of the frame tag; `0` marks a key frame.
const VP8_KEYFRAME_TAG_BIT: u8 = 0x01;
/// Dimension mask: the width/height are the low 14 bits of each 16-bit word
/// (the top 2 bits are the horizontal/vertical scale). RFC 6386 §9.1.
const VP8_DIMENSION_MASK: u16 = 0x3FFF;
/// Minimum VP8 key-frame header length: frame tag(3) + start code(3) +
/// width(2) + height(2) = 10 bytes.
const VP8_KEYFRAME_HEADER_LEN: usize = VP8_FRAME_TAG_LEN + VP8_START_CODE.len() + 4;

// --- Vorbis CodecPrivate (Vorbis I §4.2.2; Xiph lacing) ----------------------
/// Xiph-lacing header count byte value: `numPackets - 1` = 2 (three headers).
const VORBIS_LACE_COUNT: u8 = 2;
/// The Vorbis Identification-header packet type byte (Vorbis I §4.2.1): `0x01`.
const VORBIS_ID_HEADER_TYPE: u8 = 0x01;
/// The 6-byte "vorbis" signature following every header's packet-type byte.
const VORBIS_SIGNATURE: &[u8; 6] = b"vorbis";
/// Offset of `audio_channels` (u8) within the Identification header: packet
/// type(1) + "vorbis"(6) + vorbis_version(4). Vorbis I §4.2.2.
const VORBIS_ID_CHANNELS_OFFSET: usize = 1 + 6 + 4;
/// Offset of `audio_sample_rate` (u32 LE) within the Identification header:
/// after `audio_channels`. Vorbis I §4.2.2.
const VORBIS_ID_SAMPLE_RATE_OFFSET: usize = VORBIS_ID_CHANNELS_OFFSET + 1;
/// Minimum Identification-header length to read channels + sample rate.
const VORBIS_ID_MIN_LEN: usize = VORBIS_ID_SAMPLE_RATE_OFFSET + 4;

// --- VP9 vpcC defaults (WebM VP9 "profile 0 / 8-bit" when not derivable) -----
/// VPCodecConfigurationBox version (`FullBox` v1) — see [`Vp9ConfigurationBox`].
const VPCC_VERSION: u8 = 1;
/// VP9 profile 0 (8-bit 4:2:0) — the default when not derivable from CodecPrivate.
const VP9_PROFILE_0: u8 = 0;
/// VP9 level "unspecified/undefined" (0) — WebM commonly omits an explicit level.
const VP9_LEVEL_UNSPECIFIED: u8 = 0;
/// Default VP9 bit depth (8-bit).
const VP9_BIT_DEPTH_8: u8 = 8;
/// `chroma_subsampling` = 1 (4:2:0 co-located with luma), the VP9 profile-0 default.
const VP9_CHROMA_420: u8 = 1;
/// CICP `colour_primaries` = 2 (unspecified).
const CICP_UNSPECIFIED: u8 = 2;

/// A block extracted from a Cluster, before per-sample durations are assigned.
#[derive(Debug)]
struct RawBlock {
    /// 1-based Matroska track number this block belongs to.
    track_number: u64,
    /// Absolute presentation time in IR ticks (milliseconds).
    pts_ticks: i64,
    /// Whether this block is a keyframe / random-access point.
    is_sync: bool,
    /// The coded frame(s) in this block, in order. A block without lacing
    /// carries exactly one; a laced block (RFC 9559 §12 — the norm for
    /// mkvmerge-muxed audio) carries several, which must be split apart so each
    /// frame becomes its own sample rather than one concatenated blob.
    frames: Vec<Vec<u8>>,
    /// How many frames the block's lace header *declares*, recorded when the
    /// header is read so the whole-file frame budget can be checked before any
    /// frame is materialised. Equal to `frames.len()` once unlacing succeeded.
    declared_frames: usize,
}

/// A track skeleton collected while walking `Tracks`.
#[derive(Default)]
struct TrackInfo {
    /// 1-based Matroska track number (matches block track numbers).
    track_number: u64,
    /// `TrackType` (1 = video, 2 = audio).
    track_type: u64,
    /// CodecID string bytes (e.g. `V_VP9`).
    codec_id: Vec<u8>,
    /// `CodecPrivate` bytes (codec setup — e.g. the OpusHead), if present.
    codec_private: Vec<u8>,
    /// `DefaultDuration` in ns, if present.
    default_duration_ns: u64,
    /// Video `PixelWidth`, if present.
    pixel_width: u16,
    /// Video `PixelHeight`, if present.
    pixel_height: u16,
    /// Audio `Channels`, if present.
    channels: u16,
    /// Audio `SamplingFrequency` in Hz, if present.
    sampling_frequency: u32,
}

/// Demux a WebM / Matroska byte stream into a [`Media`].
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes via
/// [`Unpackage::Input`]; construct one per call with [`WebmDemux::new`].
#[derive(Debug, Default, Clone)]
pub struct WebmDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> WebmDemux<'a> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    /// Demux `input` (a whole WebM file) into a [`Media`].
    ///
    /// This is the inherent form of [`Unpackage::unpackage`]; both produce the
    /// same result. See the module docs for the pipeline and timescale.
    pub fn demux(&mut self, input: &'a [u8]) -> Result<Media> {
        let mut r = EbmlReader::new(input);
        let mut timestamp_scale_ns = DEFAULT_TIMESTAMP_SCALE_NS;
        let mut tracks: Vec<TrackInfo> = Vec::new();
        let mut blocks: Vec<RawBlock> = Vec::new();

        // Top level: EBML header + Segment(s). Only Segment carries media.
        while let Some((id, body)) = r.next_element()? {
            match id {
                EBML_HEADER => {}
                SEGMENT => {
                    Self::walk_segment(body, &mut timestamp_scale_ns, &mut tracks, &mut blocks)?;
                }
                _ => {}
            }
        }

        // Total-sample budget (allocation-amplification guard), checked against
        // the *lace headers* — the frame count each block declares — before any
        // per-frame allocation happens. Every laced frame then carries at least
        // one byte of payload (a zero-length lace is rejected in `unlace`), so N
        // bytes cannot describe more than N one-byte frames; a file declaring
        // more than that is malformed, and discovering it only after
        // materialising the frames would let a few kilobytes of headers expand
        // into millions of allocated `Vec`s first.
        let declared_frames: usize = blocks.iter().map(|b| b.declared_frames).sum();
        if declared_frames > input.len() {
            return Err(Error::InvalidInput(
                "webm: declared frame count exceeds the file's byte length",
            ));
        }

        build_media(timestamp_scale_ns, tracks, blocks)
    }

    /// Walk a `Segment` body, filling `timestamp_scale`, `tracks` and `blocks`.
    fn walk_segment(
        body: &[u8],
        timestamp_scale_ns: &mut u64,
        tracks: &mut Vec<TrackInfo>,
        blocks: &mut Vec<RawBlock>,
    ) -> Result<()> {
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element_bounded(SEGMENT_LEVEL_IDS)? {
            match id {
                INFO => Self::walk_info(child, timestamp_scale_ns)?,
                TRACKS => Self::walk_tracks(child, tracks)?,
                CLUSTER => Self::walk_cluster(child, *timestamp_scale_ns, blocks)?,
                // SeekHead, Cues, Tags, Chapters, unknown masters: skip.
                _ => {}
            }
        }
        Ok(())
    }

    /// Read `TimestampScale` out of an `Info` body.
    fn walk_info(body: &[u8], timestamp_scale_ns: &mut u64) -> Result<()> {
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            if id == TIMESTAMP_SCALE {
                *timestamp_scale_ns = read_uint(child);
            }
        }
        Ok(())
    }

    /// Walk `Tracks`, pushing one [`TrackInfo`] per `TrackEntry`.
    fn walk_tracks(body: &[u8], tracks: &mut Vec<TrackInfo>) -> Result<()> {
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            if id == TRACK_ENTRY {
                tracks.push(Self::parse_track_entry(child)?);
            }
        }
        Ok(())
    }

    /// Parse a single `TrackEntry` master into a [`TrackInfo`].
    fn parse_track_entry(body: &[u8]) -> Result<TrackInfo> {
        let mut info = TrackInfo::default();
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            match id {
                TRACK_NUMBER => info.track_number = read_uint(child),
                TRACK_TYPE => info.track_type = read_uint(child),
                CODEC_ID => info.codec_id = child.to_vec(),
                CODEC_PRIVATE => info.codec_private = child.to_vec(),
                DEFAULT_DURATION => info.default_duration_ns = read_uint(child),
                VIDEO => Self::parse_video(child, &mut info)?,
                AUDIO => Self::parse_audio(child, &mut info)?,
                _ => {}
            }
        }
        Ok(info)
    }

    /// Fill the `Video` sub-fields of a [`TrackInfo`].
    fn parse_video(body: &[u8], info: &mut TrackInfo) -> Result<()> {
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            match id {
                PIXEL_WIDTH => info.pixel_width = read_uint(child) as u16,
                PIXEL_HEIGHT => info.pixel_height = read_uint(child) as u16,
                _ => {}
            }
        }
        Ok(())
    }

    /// Fill the `Audio` sub-fields of a [`TrackInfo`].
    fn parse_audio(body: &[u8], info: &mut TrackInfo) -> Result<()> {
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            match id {
                CHANNELS => info.channels = read_uint(child) as u16,
                SAMPLING_FREQUENCY => info.sampling_frequency = read_float(child) as u32,
                _ => {}
            }
        }
        Ok(())
    }

    /// Walk a `Cluster`: read its base `Timestamp`, then each (Simple)Block.
    fn walk_cluster(
        body: &[u8],
        timestamp_scale_ns: u64,
        blocks: &mut Vec<RawBlock>,
    ) -> Result<()> {
        let mut cluster_ts: i64 = 0;
        let mut r = EbmlReader::new(body);
        while let Some((id, child)) = r.next_element()? {
            match id {
                CLUSTER_TIMESTAMP => {
                    cluster_ts = i64::try_from(read_uint(child)).map_err(|_| {
                        Error::InvalidInput("webm cluster timestamp exceeds i64 range")
                    })?;
                }
                SIMPLE_BLOCK => {
                    blocks.push(parse_block(child, cluster_ts, timestamp_scale_ns, true)?);
                }
                BLOCK_GROUP => {
                    // A BlockGroup wraps one Block. The Block carries no keyframe
                    // flag; its sync-ness is "no ReferenceBlock present in the
                    // group" (§12) — so scan the whole group before deciding.
                    let mut block_bytes: Option<&[u8]> = None;
                    let mut has_reference = false;
                    let mut g = EbmlReader::new(child);
                    while let Some((gid, gchild)) = g.next_element()? {
                        match gid {
                            BLOCK => block_bytes = Some(gchild),
                            REFERENCE_BLOCK => has_reference = true,
                            _ => {}
                        }
                    }
                    if let Some(b) = block_bytes {
                        let mut rb = parse_block(b, cluster_ts, timestamp_scale_ns, false)?;
                        rb.is_sync = !has_reference;
                        blocks.push(rb);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl<'a> Unpackage for WebmDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        self.demux(input)
    }
}

/// Parse a (Simple)Block payload into a [`RawBlock`].
///
/// Layout (RFC 9559 §12): track-number VINT, signed int16 relative timestamp,
/// flags byte, then the laced or single frame payload. `is_simple_block` selects
/// whether the keyframe flag bit is honoured (Block has no keyframe flag; its
/// sync-ness comes from being inside a keyframe-less BlockGroup, treated as
/// non-sync here).
fn parse_block(
    data: &[u8],
    cluster_ts: i64,
    timestamp_scale_ns: u64,
    is_simple_block: bool,
) -> Result<RawBlock> {
    // Track number: a VINT *value* (marker stripped).
    let (track_number, mut off) = read_vint_value(data).ok_or(Error::InvalidInput(
        "webm block: truncated track-number VINT",
    ))?;
    // int16 big-endian relative timestamp + 1 flags byte.
    if data.len() < off + 3 {
        return Err(Error::BufferTooShort {
            need: off + 3,
            have: data.len(),
            what: "webm block header (rel-ts + flags)",
        });
    }
    let rel_ts = i16::from_be_bytes([data[off], data[off + 1]]) as i64;
    let flags = data[off + 2];
    off += 3;

    let is_sync = if is_simple_block {
        flags & BLOCK_FLAG_KEYFRAME != 0
    } else {
        false
    };
    // A laced block packs several frames of the same track into one element —
    // mkvmerge's default for audio (RFC 9559 §12). Unlace them rather than
    // reject the whole file: every frame becomes its own sample, each timed
    // from the block's timestamp at the track's frame cadence (`build_media`
    // lays that out, since only the block carries a time) (r04-W40).
    //
    // The frame count is read off the lace header *first*, so the caller's
    // whole-file budget can be checked against the declared counts before any
    // frame is allocated (see `demux`).
    let declared_frames = declared_lace_count(&data[off..], flags)?;
    let frames = unlace(&data[off..], flags)?;

    // Presentation time in IR ticks (ms): (cluster_ts + rel_ts) ticks × scale(ns)
    // → ns → ms.  ns = raw_ticks × timestamp_scale_ns; ms = ns / (NS_PER_SECOND / IR_TIMESCALE).
    let raw_ticks = cluster_ts.checked_add(rel_ts).ok_or(Error::InvalidInput(
        "webm block timestamp: cluster_ts + rel_ts overflowed i64",
    ))?;
    let ns = raw_ticks.saturating_mul(timestamp_scale_ns as i64);
    let ns_per_ir_tick = (NS_PER_SECOND / IR_TIMESCALE as u64) as i64;
    let pts_ticks = ns / ns_per_ir_tick;

    Ok(RawBlock {
        track_number,
        pts_ticks,
        is_sync,
        frames,
        declared_frames,
    })
}

/// How many frames a block payload's lace header declares, without unlacing it.
///
/// A laced block's first byte is `FrameCount - 1` (RFC 9559 §12); an unlaced
/// block is one frame. Reading the count first lets a caller bound the whole
/// file's frame count before allocating anything.
///
/// `Err` when a laced payload is too short to carry its own count byte.
fn declared_lace_count(payload: &[u8], flags: u8) -> Result<usize> {
    if flags & BLOCK_FLAG_LACING_MASK == 0 {
        return Ok(1);
    }
    let count = payload.first().ok_or(Error::BufferTooShort {
        need: 1,
        have: 0,
        what: "webm laced block frame count",
    })?;
    Ok(usize::from(*count) + 1)
}

/// Split a block payload into its frames according to the block's lacing mode
/// (RFC 9559 §12), returning them in order.
///
/// Lacing lets one block carry several frames of the same track. mkvmerge emits
/// it by default for audio (Xiph lacing for Vorbis, EBML for others, fixed-size
/// where the frames are equal length), so refusing a laced block refuses every
/// ordinary mkvmerge audio file. All three modes are handled here; an
/// unrecognised lacing-mode bit pattern (the two bits cannot express a fourth
/// value) is unreachable in practice.
///
/// A declared lace size that runs past the payload is an error rather than a
/// silent truncation — the block would otherwise describe a frame it does not
/// carry.
fn unlace(payload: &[u8], flags: u8) -> Result<Vec<Vec<u8>>> {
    let mode = flags & BLOCK_FLAG_LACING_MASK;
    if mode == 0 {
        return Ok(alloc::vec![payload.to_vec()]);
    }
    // Laced blocks begin with the frame count minus one.
    let Some((&count_minus_one, rest)) = payload.split_first() else {
        return Err(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "webm laced block frame count",
        });
    };
    let frame_count = usize::from(count_minus_one) + 1;
    if frame_count > MAX_LACED_FRAMES {
        return Err(Error::InvalidInput(
            "webm block: laced frame count out of range",
        ));
    }

    // Decode the per-frame sizes, leaving `body` at the first frame's bytes.
    let mut sizes: Vec<usize> = Vec::with_capacity(frame_count);
    let mut body = rest;
    match mode {
        BLOCK_FLAG_LACING_XIPH => {
            // The first `frame_count - 1` sizes are Xiph-coded runs of 255;
            // the last frame's size is whatever remains.
            for _ in 0..frame_count - 1 {
                let mut size = 0usize;
                loop {
                    let Some((&byte, tail)) = body.split_first() else {
                        return Err(Error::BufferTooShort {
                            need: 1,
                            have: 0,
                            what: "webm Xiph lace size",
                        });
                    };
                    body = tail;
                    size = size
                        .checked_add(usize::from(byte))
                        .ok_or(Error::InvalidInput("webm Xiph lace size overflowed"))?;
                    if byte != XIPH_LACE_CONTINUATION {
                        break;
                    }
                }
                sizes.push(size);
            }
        }
        BLOCK_FLAG_LACING_EBML => {
            // The first size is a VINT; each later size is the previous one
            // plus a signed VINT delta; the last frame takes the remainder.
            let (first, used) = read_vint_generic(body)
                .ok_or(Error::InvalidInput("webm block: truncated EBML lace size"))?;
            body = &body[used..];
            let mut prev = usize::try_from(first)
                .map_err(|_| Error::InvalidInput("webm EBML lace size out of range"))?;
            sizes.push(prev);
            for _ in 1..frame_count - 1 {
                let (delta, used) = read_signed_vint(body)
                    .ok_or(Error::InvalidInput("webm block: truncated EBML lace delta"))?;
                body = &body[used..];
                let prev_i64 = i64::try_from(prev)
                    .map_err(|_| Error::InvalidInput("webm EBML lace size out of range"))?;
                let next = i128::from(prev_i64) + i128::from(delta);
                if next < 0 {
                    return Err(Error::InvalidInput("webm EBML lace size went negative"));
                }
                prev = usize::try_from(next)
                    .map_err(|_| Error::InvalidInput("webm EBML lace size out of range"))?;
                sizes.push(prev);
            }
        }
        BLOCK_FLAG_LACING_FIXED => {
            // Equal-size frames; only `frame_count - 1` sizes are implied.
            let total = body.len();
            if total % frame_count != 0 {
                return Err(Error::InvalidInput(
                    "webm block: fixed lacing payload is not a multiple of the frame count",
                ));
            }
            let each = total / frame_count;
            sizes.extend(core::iter::repeat_n(each, frame_count - 1));
        }
        _ => unreachable!("BLOCK_FLAG_LACING_MASK has exactly three non-zero values"),
    }

    // A zero-length lace would contribute an empty sample: harmless once, but
    // a hostile count byte turns it into hundreds of them from a 5-byte block,
    // and a downstream muxer then writes a file of empty samples. Real laced
    // frames always carry data, so a zero size is malformed.
    for size in &sizes {
        if *size == 0 {
            return Err(Error::InvalidInput(
                "webm block: laced frame has zero length",
            ));
        }
    }

    let mut frames = Vec::with_capacity(frame_count);
    for size in &sizes {
        if body.len() < *size {
            return Err(Error::BufferTooShort {
                need: *size,
                have: body.len(),
                what: "webm laced frame",
            });
        }
        let (frame, tail) = body.split_at(*size);
        frames.push(frame.to_vec());
        body = tail;
    }
    // The final frame is whatever is left — it has no explicit size in any mode.
    frames.push(body.to_vec());
    Ok(frames)
}

/// Read an unsigned EBML VINT as a *value* (marker bit stripped), returning it
/// with the number of bytes consumed. Unlike [`read_vint_value`] this accepts a
/// zero length-field, which is what a zero lace size is encoded as.
fn read_vint_generic(buf: &[u8]) -> Option<(u64, usize)> {
    let first = *buf.first()?;
    let mut mask = 0x80u8;
    let mut len = 1usize;
    while first & mask == 0 {
        mask >>= 1;
        len += 1;
        if len > 8 {
            return None;
        }
    }
    if buf.len() < len {
        return None;
    }
    let mut value = u64::from(first & (mask - 1));
    for &byte in &buf[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, len))
}

/// Read a signed EBML VINT for EBML lacing's size deltas: the value is
/// `read_vint_generic` minus `2^(7*len - 1) - 1` (RFC 9559 §12 / EBML RFC 8794
/// §4: the encoding is biased so it can carry negative numbers).
fn read_signed_vint(buf: &[u8]) -> Option<(i64, usize)> {
    let first = *buf.first()?;
    let mut mask = 0x80u8;
    let mut len = 1usize;
    while first & mask == 0 {
        mask >>= 1;
        len += 1;
        if len > 8 {
            return None;
        }
    }
    let (raw, used) = read_vint_generic(buf)?;
    let signed = i64::try_from(raw).ok()? - ebml_lace_delta_bias(len);
    Some((signed, used))
}

#[cfg(test)]
thread_local! {
    /// Work counter for [`build_media`] (r04-O9): every block examination, in the
    /// one-pass partition and in the per-track gather, so a reintroduced
    /// per-track rescan of all blocks (O(tracks x blocks)) raises it.
    static BLOCKS_VISITED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Assemble the collected tracks + blocks into a [`Media`].
///
/// One [`Track`] per elementary stream whose CodecID we support, samples in
/// decode order (blocks are stored in file order, which for these single-Cluster
/// / monotonic fixtures is decode order per track). Per-sample duration is the
/// delta to the next block of the same track; the final sample reuses the
/// previous delta, or `DefaultDuration` (ns → ms) when only one block exists.
fn build_media(
    timestamp_scale_ns: u64,
    tracks: Vec<TrackInfo>,
    mut blocks: Vec<RawBlock>,
) -> Result<Media> {
    let _ = timestamp_scale_ns;
    // Partition the blocks by track once (file order preserved within a track)
    // instead of rescanning every block for every track (r04-O9). The frames
    // are moved out of each block, never cloned.
    let mut by_track: BTreeMap<u64, Vec<RawBlock>> = BTreeMap::new();
    for b in blocks.drain(..) {
        #[cfg(test)]
        BLOCKS_VISITED.with(|c| c.set(c.get() + 1));
        by_track.entry(b.track_number).or_default().push(b);
    }
    let mut out_tracks: Vec<Track> = Vec::new();
    let mut track_id: u32 = 1;

    for info in &tracks {
        // `DefaultDuration` (ns per frame) in this track's IR ticks, when the
        // TrackEntry declares one; 0 means "not declared". Converted by
        // *rounding* rather than by integer-dividing nanoseconds into
        // milliseconds first: 23 219 954 ns (an AAC frame at 44.1 kHz) is
        // 23.219954 ms, and truncating to 23 ms loses 219 954 ns per frame —
        // ~220 µs of cumulative drift per frame across a laced run.
        let default_dur_ir = ns_to_ir_ticks(info.default_duration_ns);

        // Gather this track's blocks in file (decode) order. Each block is kept
        // with its own timestamp and its frames, so the per-frame timestamps can
        // be laid out below once the nominal frame duration is known.
        let mut timeline: Vec<BlockSpan> = Vec::new();
        let mut samples: Vec<Sample> = Vec::new();
        let mut sync: Vec<bool> = Vec::new();
        let mut payloads: Vec<Vec<u8>> = Vec::new();
        // The frames are *moved* out of the block rather than cloned: a laced
        // block can carry up to `MAX_LACED_FRAMES` frames, and cloning each one
        // would double the peak allocation for no reason. `blocks` is owned by
        // this function, and each block belongs to exactly one track, so taking
        // its frames is safe.
        for mut b in by_track.remove(&info.track_number).unwrap_or_default() {
            #[cfg(test)]
            BLOCKS_VISITED.with(|c| c.set(c.get() + 1));
            timeline.push(BlockSpan {
                pts_ticks: b.pts_ticks,
                first_frame: payloads.len(),
                frame_count: b.frames.len(),
            });
            sync.extend(core::iter::repeat_n(b.is_sync, b.frames.len()));
            payloads.append(&mut b.frames);
        }
        if payloads.is_empty() {
            continue;
        }
        // Nominal per-frame duration in IR ticks: `DefaultDuration` when the
        // TrackEntry declares one, else the interval from a block to its
        // successor spread evenly over the frames in between (which is what a
        // muxer that omits `DefaultDuration` leaves as the only information),
        // else the track's own last known interval.
        let nominal = if default_dur_ir > 0 {
            default_dur_ir
        } else {
            derive_nominal_frame_duration(&timeline)
        };
        let pts = lay_out_block_timestamps(&timeline, nominal);

        // Codec config: resolved from the TrackEntry plus, for codecs whose
        // geometry lives in-band (VP8), the first sync sample's frame header.
        // Skip unsupported CodecIDs (never fatal).
        let first_sync = sync
            .iter()
            .position(|&s| s)
            .map(|i| payloads[i].as_slice())
            .unwrap_or(payloads[0].as_slice());
        let Some(codec) = codec_config_for(info, first_sync)? else {
            continue;
        };
        let (config, nal_length_size) = codec;

        let n = payloads.len();
        for i in 0..n {
            // A delta between two IR timestamps cannot exceed the `u32` sample
            // duration field, but convert checked rather than with `as` so an
            // absurd span is an error instead of a wrapped duration.
            let delta = if i + 1 < n {
                pts[i + 1] - pts[i]
            } else if n >= 2 {
                pts[i] - pts[i - 1]
            } else {
                i64::try_from(nominal).unwrap_or(i64::MAX)
            };
            let duration = u32::try_from(delta.max(0))
                .map_err(|_| Error::InvalidInput("webm: sample duration does not fit the IR"))?;
            // Absolute dts/pts (media plane step 2c): WebM carries only a
            // presentation time per block (RFC 9559 §12) with no separate
            // decode-time field, so dts == pts (WebM's VP8/VP9/Opus/Vorbis
            // scope here has no B-frame reordering to express).
            // The block's NAL prefixes carry whatever length size the
            // `CodecPrivate` `avcC`/`hvcC` declared (§5.3.3); rewrite them to
            // the crate's 4-byte form so keyframe detection and the TS/RTP
            // writers do not read garbage lengths (r04-W43).
            let data = core::mem::take(&mut payloads[i]);
            let data = match nal_length_size {
                Some(size) => normalise_nal_length_size(&data, size)?,
                None => data,
            };
            samples.push(Sample {
                data: data.into(),
                dts: Some(pts[i]),
                pts: Some(pts[i]),
                duration: Some(duration),
                flags: crate::ir::SampleFlags::new(sync[i]),
                provenance: None,
            });
        }

        // Anchor at the first block's absolute pts, kept in lockstep with
        // `samples[0].dts` (media plane step 2c) — WebM does carry a real
        // absolute clock, unlike the demuxers with no anchor at all.
        let anchor = pts.first().map(|&p| p.max(0) as u64).unwrap_or(0);
        out_tracks.push(Track::new_at(
            TrackSpec::new(track_id, IR_TIMESCALE, config),
            samples,
            anchor,
        ));
        track_id += 1;
    }

    Ok(Media::new(out_tracks, IR_TIMESCALE))
}

/// One block's contribution to a track's frame timeline: when the block starts,
/// where its first frame lands in the flattened frame list, and how many frames
/// it contributed. Holds the values rather than a borrow of the block, so the
/// frames themselves can be moved out of their blocks as they are gathered.
struct BlockSpan {
    /// The block's own presentation time, in IR ticks.
    pts_ticks: i64,
    /// Index of this block's first frame in the flattened frame list.
    first_frame: usize,
    /// How many frames the block contributed.
    frame_count: usize,
}

/// Convert a `DefaultDuration` in nanoseconds to this crate's IR ticks,
/// **rounding to nearest** rather than truncating.
///
/// The arithmetic is done in `u128` nanoseconds against the IR timescale, so it
/// stays exact for any input; an integer divide of nanoseconds by 1e6 first
/// (i.e. via whole milliseconds) loses up to a millisecond per frame, which
/// accumulates across every frame of a laced run.
pub fn ns_to_ir_ticks(ns: u64) -> u64 {
    let ticks = u128::from(ns) * u128::from(IR_TIMESCALE);
    // Round-half-up: add half a second's worth of nanoseconds before dividing.
    let half = NS_PER_SECOND as u128 / 2;
    u64::try_from((ticks + half) / NS_PER_SECOND as u128).unwrap_or(u64::MAX)
}

/// The nominal per-frame duration (IR ticks) for a track whose TrackEntry
/// declares no `DefaultDuration`.
///
/// A block's frames must fit in the interval from that block's own timestamp to
/// the next block's, so the per-frame duration is that interval divided by the
/// number of frames the earlier block contributes (the later block's own first
/// frame defines the far boundary, not part of the gap). The first pair with a
/// positive gap wins. A track with fewer than two blocks — or whose blocks all
/// share a timestamp — has no interval to derive from and reports 0, and the
/// caller then falls back to one tick so no frame is left zero-length.
///
/// Neither FFmpeg (`matroskadec.c`) nor mkvtoolnix derives anything here: both
/// leave every lace of such a block with duration 0, so the frames' real timing
/// is recovered downstream from the codec's own frame headers. Doing the same
/// would be exactly the r04-W40 defect (a laced block collapsing onto one
/// instant), and this crate's IR has no downstream parser to repair it.
fn derive_nominal_frame_duration(timeline: &[BlockSpan]) -> u64 {
    for pair in timeline.windows(2) {
        let (cur, next) = (&pair[0], &pair[1]);
        let frames = (next.first_frame - cur.first_frame) as u64;
        let gap = next.pts_ticks.saturating_sub(cur.pts_ticks);
        if frames > 0 && gap > 0 {
            return (gap as u64) / frames;
        }
    }
    0
}

/// Absolute IR-tick timestamps for every frame of every block in `timeline`
/// (which holds `(block, first-frame index)` pairs in file order).
///
/// Matroska times a *block*, not the frames inside it (RFC 9559 §12), so a
/// laced block's frames are consecutive from the block's own time: frame `i`
/// starts `i * nominal` ticks later. Leaving them all at the block timestamp
/// gives every frame but the last a zero duration and piles the block onto one
/// instant. `nominal == 0` (a track with a single frame and no declared
/// duration) falls back to one tick so that no non-final frame is zero-length.
fn lay_out_block_timestamps(timeline: &[BlockSpan], nominal: u64) -> Vec<i64> {
    let nominal = if nominal == 0 { 1 } else { nominal };
    let total: usize = timeline
        .last()
        .map(|b| b.first_frame + b.frame_count)
        .unwrap_or(0);
    let mut out = alloc::vec![0i64; total];
    for block in timeline {
        for i in 0..block.frame_count {
            out[block.first_frame + i] = block
                .pts_ticks
                .saturating_add((i as i64).saturating_mul(nominal as i64));
        }
    }
    out
}

/// Map a [`TrackInfo`] to a [`CodecConfig`] plus, for the length-prefixed
/// codecs (AVC/HEVC), the NAL length-prefix size its `CodecPrivate` declares —
/// or `None` for an unsupported CodecID.
///
/// `first_frame` is the first sync sample's coded bytes (used to decode the VP8
/// key-frame header dimensions; ignored for the other codecs).
///
/// The length size is returned so [`build_media`] can normalise each block's
/// NAL prefixes to the crate's 4-byte form: the pipeline downstream is fixed at
/// 4 bytes, while a source `avcC`/`hvcC` may declare 1, 2 or 4 (§5.3.3).
fn codec_config_for(
    info: &TrackInfo,
    first_frame: &[u8],
) -> Result<Option<(CodecConfig, Option<usize>)>> {
    if info.track_type == TRACK_TYPE_VIDEO && info.codec_id == CODEC_V_VP9 {
        Ok(Some((vp9_config(info), None)))
    } else if info.track_type == TRACK_TYPE_VIDEO && info.codec_id == CODEC_V_VP8 {
        Ok(Some((vp8_config(first_frame)?, None)))
    } else if info.track_type == TRACK_TYPE_VIDEO && info.codec_id == CODEC_V_AVC {
        Ok(Some(avc_config(info)?))
    } else if info.track_type == TRACK_TYPE_VIDEO && info.codec_id == CODEC_V_HEVC {
        Ok(Some(hevc_config(info)?))
    } else if info.track_type == TRACK_TYPE_AUDIO && info.codec_id == CODEC_A_OPUS {
        Ok(Some((opus_config(info)?, None)))
    } else if info.track_type == TRACK_TYPE_AUDIO && info.codec_id == CODEC_A_VORBIS {
        Ok(Some((vorbis_config(info)?, None)))
    } else if info.track_type == TRACK_TYPE_AUDIO && info.codec_id == CODEC_A_AAC {
        Ok(Some((
            aac_config_from_asc_bytes(info.codec_private.clone())?,
            None,
        )))
    } else {
        Ok(None)
    }
}

/// Build a [`CodecConfig::Avc`] from an H.264 [`TrackInfo`]: `CodecPrivate` is
/// the raw `AVCDecoderConfigurationRecord` (ISO/IEC 14496-15 §5.3.3); the coded
/// dimensions come from the `Video` element (§27, `PixelWidth`/`PixelHeight`) —
/// mirrors [`crate::mkv_mux::MkvMux`]'s inverse `CodecPrivate` emission.
///
/// Returns the config together with the record's NAL length-prefix size
/// (`lengthSizeMinusOne + 1`, §5.3.3.1.2 / §5.3.3.3). The emitted record's
/// `lengthSizeMinusOne` is rewritten to the crate's canonical 4-byte form
/// ([`NAL_LENGTH_SIZE_MINUS_ONE`]), because `build_media` normalises each
/// block's NAL prefixes with [`normalise_nal_length_size`] — an init segment
/// that still declared the source's size would describe lengths the samples do
/// not have.
fn avc_config(info: &TrackInfo) -> Result<(CodecConfig, Option<usize>)> {
    let mut record = AVCDecoderConfigurationRecord::parse(&info.codec_private)?;
    let length_size = usize::from(record.length_size_minus_one) + 1;
    record.length_size_minus_one = NAL_LENGTH_SIZE_MINUS_ONE;
    Ok((
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: info.pixel_width,
            height: info.pixel_height,
        },
        Some(length_size),
    ))
}

/// Build a [`CodecConfig::Hevc`] from an H.265 [`TrackInfo`]: `CodecPrivate` is
/// the raw `HEVCDecoderConfigurationRecord` (ISO/IEC 14496-15 §8.3.3.1); the
/// coded dimensions come from the `Video` element, as [`avc_config`]. Also
/// returns the record's NAL length-prefix size (`lengthSizeMinusOne + 1`), and
/// rewrites the emitted record's `lengthSizeMinusOne` to the canonical 4-byte
/// form for the same reason as [`avc_config`].
fn hevc_config(info: &TrackInfo) -> Result<(CodecConfig, Option<usize>)> {
    let mut record = HEVCDecoderConfigurationRecord::parse(&info.codec_private)?;
    let length_size = usize::from(record.length_size_minus_one) + 1;
    record.length_size_minus_one = NAL_LENGTH_SIZE_MINUS_ONE;
    Ok((
        CodecConfig::Hevc {
            config: HEVCConfigurationBox::new(record),
            width: info.pixel_width,
            height: info.pixel_height,
        },
        Some(length_size),
    ))
}

/// Build a [`CodecConfig::Vp8`] by decoding the VP8 key-frame header (RFC 6386
/// §9.1 / §19.1) of the first key frame.
///
/// Layout: a 3-byte uncompressed frame tag whose bit `[0]` (`key_frame`) is `0`
/// for a key frame, then the 3-byte start code `0x9D 01 2A`, then two 16-bit
/// little-endian words carrying `width`/`height` in their low 14 bits (the top
/// two bits are the horizontal/vertical scale). See
/// `docs/codec/vp8-vorbis-webm.md`.
fn vp8_config(first_frame: &[u8]) -> Result<CodecConfig> {
    if first_frame.len() < VP8_KEYFRAME_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: VP8_KEYFRAME_HEADER_LEN,
            have: first_frame.len(),
            what: "VP8 key-frame header",
        });
    }
    // Frame tag is 24-bit little-endian; bit [0] of byte 0 is `key_frame`.
    if first_frame[0] & VP8_KEYFRAME_TAG_BIT != 0 {
        return Err(Error::InvalidValue {
            field: "VP8 key_frame",
            value: (first_frame[0] & VP8_KEYFRAME_TAG_BIT) as u64,
            reason: "first VP8 frame is not a key frame (key_frame bit != 0)",
        });
    }
    let start = &first_frame[VP8_FRAME_TAG_LEN..VP8_FRAME_TAG_LEN + VP8_START_CODE.len()];
    if start != VP8_START_CODE {
        return Err(Error::InvalidValue {
            field: "VP8 start code",
            value: u32::from_be_bytes([0, start[0], start[1], start[2]]) as u64,
            reason: "VP8 key-frame start code is not 0x9D012A",
        });
    }
    let d = VP8_FRAME_TAG_LEN + VP8_START_CODE.len();
    let width = u16::from_le_bytes([first_frame[d], first_frame[d + 1]]) & VP8_DIMENSION_MASK;
    let height = u16::from_le_bytes([first_frame[d + 2], first_frame[d + 3]]) & VP8_DIMENSION_MASK;
    Ok(CodecConfig::Vp8 { width, height })
}

/// Build a [`CodecConfig::Vorbis`] from a Vorbis [`TrackInfo`].
///
/// The `CodecPrivate` is stored verbatim (the three Xiph-laced setup headers).
/// The Identification header (Vorbis I §4.2.2) is located past the Xiph lacing
/// (byte 0 = `numPackets - 1` = 2, then the laced lengths of the first two
/// headers) and decoded for `audio_channels` (u8) + `audio_sample_rate` (u32
/// LE). See `docs/codec/vp8-vorbis-webm.md`.
fn vorbis_config(info: &TrackInfo) -> Result<CodecConfig> {
    let cp = &info.codec_private;
    let id = vorbis_id_header(cp)?;

    // Identification header: packet-type(1) + "vorbis"(6) signature.
    if id.len() < VORBIS_ID_MIN_LEN {
        return Err(Error::BufferTooShort {
            need: VORBIS_ID_MIN_LEN,
            have: id.len(),
            what: "Vorbis identification header",
        });
    }
    if id[0] != VORBIS_ID_HEADER_TYPE {
        return Err(Error::InvalidValue {
            field: "Vorbis header packet type",
            value: id[0] as u64,
            reason: "first Vorbis header is not the identification header (type 0x01)",
        });
    }
    if &id[1..1 + VORBIS_SIGNATURE.len()] != VORBIS_SIGNATURE {
        return Err(Error::InvalidInput(
            "Vorbis identification header missing the \"vorbis\" signature",
        ));
    }
    let channels = id[VORBIS_ID_CHANNELS_OFFSET] as u16;
    let sample_rate = u32::from_le_bytes([
        id[VORBIS_ID_SAMPLE_RATE_OFFSET],
        id[VORBIS_ID_SAMPLE_RATE_OFFSET + 1],
        id[VORBIS_ID_SAMPLE_RATE_OFFSET + 2],
        id[VORBIS_ID_SAMPLE_RATE_OFFSET + 3],
    ]);

    Ok(CodecConfig::Vorbis {
        codec_private: cp.clone(),
        channels,
        sample_rate,
    })
}

/// Slice the Vorbis Identification header out of the Xiph-laced `CodecPrivate`.
///
/// Xiph lacing (Vorbis I §4.2.2): byte 0 = `numPackets - 1` = 2; then the length
/// of the first two headers, each as a run of bytes summed while the byte is
/// `0xFF`; the third header's length is the remainder. The identification header
/// is the first packet, so its length is the first laced length and it starts
/// right after the lacing table.
fn vorbis_id_header(cp: &[u8]) -> Result<&[u8]> {
    if cp.is_empty() {
        return Err(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "Vorbis CodecPrivate (Xiph lacing count)",
        });
    }
    if cp[0] != VORBIS_LACE_COUNT {
        return Err(Error::InvalidValue {
            field: "Vorbis CodecPrivate lacing count",
            value: cp[0] as u64,
            reason: "expected numPackets-1 == 2 (three Xiph-laced Vorbis headers)",
        });
    }
    // Read the laced lengths of the first two headers.
    let mut pos = 1usize;
    let mut lengths = [0usize; 2];
    for len in lengths.iter_mut() {
        loop {
            let b = *cp.get(pos).ok_or(Error::BufferTooShort {
                need: pos + 1,
                have: cp.len(),
                what: "Vorbis CodecPrivate Xiph lacing length",
            })?;
            pos += 1;
            *len += b as usize;
            if b != 0xFF {
                break;
            }
        }
    }
    // The identification header is the first packet, immediately after the table.
    let id_start = pos;
    let id_end = id_start
        .checked_add(lengths[0])
        .filter(|&e| e <= cp.len())
        .ok_or(Error::BufferTooShort {
            need: id_start + lengths[0],
            have: cp.len(),
            what: "Vorbis identification header body",
        })?;
    Ok(&cp[id_start..id_end])
}

/// Build a [`CodecConfig::Vp9`] from a VP9 [`TrackInfo`].
///
/// WebM stores no `vpcC` in CodecPrivate for VP9, so a profile-0 / 8-bit / 4:2:0
/// `vpcC` is synthesised (documented default per `docs/webm/ebml-matroska.md`);
/// the pixel dimensions come from the `Video` element.
fn vp9_config(info: &TrackInfo) -> CodecConfig {
    let config = Vp9ConfigurationBox {
        version: VPCC_VERSION,
        flags: 0,
        profile: VP9_PROFILE_0,
        level: VP9_LEVEL_UNSPECIFIED,
        bit_depth: VP9_BIT_DEPTH_8,
        chroma_subsampling: VP9_CHROMA_420,
        video_full_range_flag: false,
        colour_primaries: CICP_UNSPECIFIED,
        transfer_characteristics: CICP_UNSPECIFIED,
        matrix_coefficients: CICP_UNSPECIFIED,
        codec_initialization_data: Vec::new(),
    };
    CodecConfig::Vp9 {
        config,
        width: info.pixel_width,
        height: info.pixel_height,
    }
}

/// Build a [`CodecConfig::Opus`] from an Opus [`TrackInfo`], parsing the
/// `OpusHead` identification header carried in `CodecPrivate` (RFC 7845 §5.1).
///
/// The `dOps` `OpusSpecificBox` fields are populated directly from the OpusHead
/// (version, channel count, pre-skip, input sample rate, output gain, channel
/// mapping). The `OpusHead` magic is validated — a missing/short/incorrect
/// header is a hard error, not a silent default.
fn opus_config(info: &TrackInfo) -> Result<CodecConfig> {
    let cp = &info.codec_private;
    if cp.len() < OPUS_HEAD_MIN_LEN {
        return Err(Error::BufferTooShort {
            need: OPUS_HEAD_MIN_LEN,
            have: cp.len(),
            what: "Opus CodecPrivate (OpusHead)",
        });
    }
    if &cp[0..8] != OPUS_HEAD_MAGIC {
        return Err(Error::InvalidValue {
            field: "OpusHead magic",
            value: u64::from_be_bytes([cp[0], cp[1], cp[2], cp[3], cp[4], cp[5], cp[6], cp[7]]),
            reason: "Opus CodecPrivate does not start with the \"OpusHead\" signature",
        });
    }
    // OpusHead (RFC 7845 §5.1) is little-endian; dOps is big-endian but the
    // typed OpusSpecificBox holds decoded scalar values, so byte-order is handled
    // here on read.
    let version = cp[8];
    let output_channel_count = cp[9];
    let pre_skip = u16::from_le_bytes([cp[10], cp[11]]);
    let input_sample_rate = u32::from_le_bytes([cp[12], cp[13], cp[14], cp[15]]);
    let output_gain = i16::from_le_bytes([cp[16], cp[17]]);
    let channel_mapping_family = cp[18];

    let channel_mapping = if channel_mapping_family != 0 {
        // Channel-mapping table: StreamCount(1) CoupledCount(1) ChannelMapping[Nch].
        let need = OPUS_HEAD_MIN_LEN + 2 + output_channel_count as usize;
        if cp.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: cp.len(),
                what: "OpusHead channel-mapping table",
            });
        }
        let stream_count = cp[19];
        let coupled_count = cp[20];
        let map_start = 21;
        let map_end = map_start + output_channel_count as usize;
        Some(crate::opus::ChannelMappingTable {
            stream_count,
            coupled_count,
            channel_mapping: cp[map_start..map_end].to_vec(),
        })
    } else {
        None
    };

    let dops = OpusSpecificBox {
        version,
        output_channel_count,
        pre_skip,
        input_sample_rate,
        output_gain,
        channel_mapping_family,
        channel_mapping,
    };
    Ok(CodecConfig::Opus {
        config: dops,
        channel_count: output_channel_count as u16,
        sample_rate: OPUS_OUTPUT_SAMPLE_RATE,
        sample_size: AUDIO_SAMPLE_SIZE,
    })
}

// ---------------------------------------------------------------------------
// EBML framing (RFC 8794 §4)
// ---------------------------------------------------------------------------

/// A cursor over an EBML element list, yielding `(element_id, body_bytes)` pairs.
struct EbmlReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> EbmlReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Read the next `(element_id, body)` element, or `None` at end of buffer.
    ///
    /// An element is `ID (VINT, marker kept) | size (VINT, marker stripped) |
    /// body[size]`. An "unknown size" (all-ones data bits) element runs to the
    /// end of the enclosing buffer. No sibling-boundary IDs are known at this
    /// call site, so this is only correct when nothing can legitimately follow
    /// the element within `self.buf` (top-level EBML/Segment scanning, and any
    /// context where an unknown-size child isn't expected to have a sibling in
    /// the same buffer) — [`Self::next_element_bounded`] is the fix for the
    /// case where it can (C10, #1011).
    fn next_element(&mut self) -> Result<Option<(u32, &'a [u8])>> {
        self.next_element_bounded(&[])
    }

    /// Like [`Self::next_element`], but an **unknown-size** element's body
    /// ends at the first later position that decodes as a valid EBML element
    /// header whose ID is in `boundary_ids`, instead of running to the end of
    /// `self.buf` (C10, #1011: a live-written Cluster is unknown-size, and
    /// without a boundary its "body" silently swallowed every later Cluster
    /// in the Segment). `boundary_ids` should list every element ID valid at
    /// the *caller's own* nesting level (e.g. [`SEGMENT_LEVEL_IDS`] when
    /// walking a Segment's children) — RFC 8794 §6.2 ends an unknown-size
    /// element at the first element that is not one of its own descendants,
    /// and any of those sibling IDs reappearing can only mean that.
    fn next_element_bounded(&mut self, boundary_ids: &[u32]) -> Result<Option<(u32, &'a [u8])>> {
        if self.pos >= self.buf.len() {
            return Ok(None);
        }
        let rest = &self.buf[self.pos..];
        let (id, id_len) =
            read_element_id(rest).ok_or(Error::InvalidInput("webm: truncated element ID"))?;
        let after_id = &rest[id_len..];
        let (size, size_len, unknown) = read_element_size(after_id)
            .ok_or(Error::InvalidInput("webm: truncated element size"))?;
        let body_start = self.pos + id_len + size_len;
        let body_end = if unknown {
            find_sibling_boundary(self.buf, body_start, boundary_ids).unwrap_or(self.buf.len())
        } else {
            let end = body_start + size as usize;
            if end > self.buf.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: self.buf.len(),
                    what: "webm element body",
                });
            }
            end
        };
        let body = &self.buf[body_start..body_end];
        self.pos = body_end;
        Ok(Some((id, body)))
    }
}

/// Scan `buf[from..]` for the first byte offset that decodes as a valid EBML
/// element header (ID + size, with the size either "unknown" itself or fully
/// in-bounds) whose ID is a member of `boundary_ids`. Returns `None` if
/// `boundary_ids` is empty or no such position exists before the end of
/// `buf`. Used only to bound an **unknown-size** element (C10, #1011); a
/// well-formed stream's element headers don't occur by chance inside coded
/// media, but a byte-for-byte match plus a structurally valid trailing size
/// field makes a false positive exceedingly unlikely, and is the same class
/// of resync heuristic real Matroska demuxers use for unknown-size Clusters.
fn find_sibling_boundary(buf: &[u8], from: usize, boundary_ids: &[u32]) -> Option<usize> {
    if boundary_ids.is_empty() {
        return None;
    }
    let mut p = from;
    while p < buf.len() {
        if let Some((id, id_len)) = read_element_id(&buf[p..])
            && boundary_ids.contains(&id)
            && let Some((size, size_len, unknown)) = read_element_size(&buf[p + id_len..])
        {
            let body_start = p + id_len + size_len;
            if unknown || body_start + size as usize <= buf.len() {
                return Some(p);
            }
        }
        p += 1;
    }
    None
}

/// Read an EBML **element ID** (VINT with the length-marker bits *kept*).
///
/// Returns `(id, byte_len)`. IDs are 1–4 bytes; the width is the leading-zero
/// count of the first byte + 1.
fn read_element_id(buf: &[u8]) -> Option<(u32, usize)> {
    let first = *buf.first()?;
    if first == 0 {
        return None; // 4+ leading zero bytes: not a valid 1–4-byte ID.
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || buf.len() < len {
        return None;
    }
    let mut id: u32 = 0;
    for &b in &buf[..len] {
        id = (id << 8) | b as u32;
    }
    Some((id, len))
}

/// Read an EBML **element size** (VINT with the length-marker bit *stripped*).
///
/// Returns `(value, byte_len, is_unknown)`. `is_unknown` is set when all data
/// bits are 1 (the reserved "unknown size" encoding).
fn read_element_size(buf: &[u8]) -> Option<(u64, usize, bool)> {
    let (value, len, all_ones) = read_vint(buf)?;
    Some((value, len, all_ones))
}

/// Read a VINT, returning `(data_value, byte_len, all_data_bits_set)`.
///
/// The width is the leading-zero count of the first byte + 1 (1–8 bytes). The
/// first `1` bit is the length marker and is stripped; the remaining bits are
/// the value. `all_data_bits_set` distinguishes the "unknown size" reserved
/// value from a genuine maximal value.
fn read_vint(buf: &[u8]) -> Option<(u64, usize, bool)> {
    let first = *buf.first()?;
    if first == 0 {
        return None; // width > 8: unsupported here.
    }
    let len = first.leading_zeros() as usize + 1;
    if buf.len() < len {
        return None;
    }
    // Strip the marker bit (the highest set bit of the first byte). For width 8
    // the entire first byte is the marker, so its data contribution is 0.
    let first_mask: u8 = if len >= 8 { 0 } else { 0xFF >> len };
    let mut value = (first & first_mask) as u64;
    for &b in &buf[1..len] {
        value = (value << 8) | b as u64;
    }
    // Maximum representable data value for this width (all data bits set).
    let data_bits = 7 * len; // 7 per byte after stripping one marker bit.
    let max = if data_bits >= 64 {
        u64::MAX
    } else {
        (1u64 << data_bits) - 1
    };
    Some((value, len, value == max))
}

/// Read a VINT **value** (marker stripped), returning `(value, byte_len)`.
///
/// Used for a (Simple)Block track number, where only the value matters.
fn read_vint_value(buf: &[u8]) -> Option<(u64, usize)> {
    read_vint(buf).map(|(v, len, _)| (v, len))
}

/// Read a big-endian unsigned integer element body (1–8 bytes) as a `u64`.
///
/// EBML uint leaf elements are stored big-endian with the encoded length; a
/// shorter body just means fewer significant bytes.
fn read_uint(body: &[u8]) -> u64 {
    let mut v: u64 = 0;
    for &b in body.iter().take(8) {
        v = (v << 8) | b as u64;
    }
    v
}

/// Read an EBML `float` element body (4 or 8 bytes, big-endian IEEE 754).
///
/// Returns `0.0` for any other length (absent / malformed) — callers only use
/// this for `SamplingFrequency`, where a bad value degrades to a 0 rate.
fn read_float(body: &[u8]) -> f64 {
    match body.len() {
        4 => f32::from_be_bytes([body[0], body[1], body[2], body[3]]) as f64,
        8 => f64::from_be_bytes([
            body[0], body[1], body[2], body[3], body[4], body[5], body[6], body[7],
        ]),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// r04-O9: `build_media` rescanned every block once per track (O(T x B)).
    /// 8 tracks x 1000 blocks: 8000 block examinations before; now one partition
    /// pass plus one gather pass over each block (2000), independent of the track
    /// count. The counter sits in both loops, so a reintroduced per-track rescan
    /// (counted in its loop) fails the bound.
    #[test]
    fn build_media_examines_each_block_once_not_once_per_track() {
        const TRACKS: u64 = 8;
        const BLOCKS: usize = 1000;
        let tracks: Vec<TrackInfo> = (1..=TRACKS)
            .map(|n| TrackInfo {
                track_number: n,
                codec_id: b"X_UNSUPPORTED".to_vec(),
                ..TrackInfo::default()
            })
            .collect();
        let blocks: Vec<RawBlock> = (0..BLOCKS)
            .map(|i| RawBlock {
                track_number: (i as u64 % TRACKS) + 1,
                pts_ticks: i as i64,
                is_sync: true,
                frames: vec![vec![0u8; 4]],
                declared_frames: 1,
            })
            .collect();
        BLOCKS_VISITED.with(|c| c.set(0));
        build_media(1_000_000, tracks, blocks).unwrap();
        let visited = BLOCKS_VISITED.with(core::cell::Cell::get);
        assert_eq!(
            visited,
            2 * BLOCKS,
            "partition + gather, not per-track rescans"
        );
    }

    #[test]
    fn vint_one_byte() {
        // 0x81 = 1000_0001 → width 1, value 1 (not the all-ones "unknown" value).
        assert_eq!(read_vint(&[0x81]), Some((1, 1, false)));
        // 0xFF = 1111_1111 → width 1, value 127 = all data bits set (unknown-size).
        assert_eq!(read_vint(&[0xFF]), Some((127, 1, true)));
    }

    #[test]
    fn vint_two_byte() {
        // 0x40 0x02 = width 2, value 2.
        assert_eq!(read_vint(&[0x40, 0x02]), Some((2, 2, false)));
    }

    #[test]
    fn element_id_segment() {
        // Segment ID 0x1853_8067 is 4 bytes with the marker kept.
        assert_eq!(
            read_element_id(&[0x18, 0x53, 0x80, 0x67]),
            Some((SEGMENT, 4))
        );
    }

    #[test]
    fn element_id_track_entry() {
        // TrackEntry ID 0xAE is 1 byte with the marker kept.
        assert_eq!(read_element_id(&[0xAE]), Some((TRACK_ENTRY, 1)));
    }

    #[test]
    fn uint_be() {
        assert_eq!(read_uint(&[0x0F, 0x42, 0x40]), 1_000_000);
    }

    #[test]
    fn laced_block_is_unlaced() {
        // track-number VINT (0x81), rel-ts int16 (0,0), flags with Xiph lacing
        // (0x02), frame count minus one (0x01 = two frames), one lace size
        // (0x02), then the two frames.
        let block = [0x81u8, 0x00, 0x00, 0x02, 0x01, 0x02, 0xAA, 0xBB, 0xCC, 0xDD];
        let parsed = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).expect("parse");
        assert_eq!(
            parsed.frames,
            alloc::vec![alloc::vec![0xAA, 0xBB], alloc::vec![0xCC, 0xDD]],
            "a Xiph-laced block splits into its frames, sized per the lace run"
        );
    }

    #[test]
    fn unlaced_block_is_one_frame() {
        // No lacing bits: the payload after the flags is the single frame.
        let block = [0x81u8, 0x00, 0x00, 0x80, 0xAA, 0xBB];
        let parsed = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).expect("parse");
        assert_eq!(parsed.frames, alloc::vec![alloc::vec![0xAA, 0xBB]]);
    }

    #[test]
    fn ebml_lacing_sizes_each_frame_from_its_delta() {
        // Frame count minus one = 2 (three frames); first size VINT 0x83 (= 3);
        // then two signed VINT deltas. A one-byte signed VINT has bias
        // 2^(7-1)-1 = 63, so +1 encodes as 0x80|(1+63) = 0xC0. Frames are
        // therefore 3, 3+1=4, and the remainder 1 byte.
        let bytes = [
            0x81u8, 0x00, 0x00, 0x06, 0x02, // track, ts, flags, frame count
            0x83, 0xC0, // first size 3, then one delta of +1
            0xAA, 0xAA, 0xAA, 0xBB, 0xBB, 0xBB, 0xBB, 0xCC, // 3 + 4 + 1 bytes
        ];
        let parsed = parse_block(&bytes, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).expect("parse");
        assert_eq!(
            parsed
                .frames
                .iter()
                .map(|f| f.len())
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![3, 4, 1],
            "EBML lacing sizes each frame from the running VINT delta"
        );
    }

    #[test]
    fn fixed_lacing_needs_a_whole_multiple() {
        // Frame count minus one = 1 (two frames) but 5 payload bytes is odd.
        let block = [0x81u8, 0x00, 0x00, 0x04, 0x01, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        let err = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "a fixed-laced payload that is not a multiple of the frame count is rejected, \
             never silently truncated"
        );
    }

    #[test]
    fn zero_length_laced_frames_are_rejected() {
        // Fixed lacing, count byte 0xFF (256 frames) and no payload: every
        // frame's size is 0. Accepting this yields 256 empty samples from a
        // 5-byte block, and (before r04-W40's timing fix) tens of millions of
        // them once the frames were materialised. It is a malformed block.
        let block = [0x81u8, 0x00, 0x00, 0x04, 0xFF];
        let err = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "a zero-length laced frame must be rejected, got {err:?}"
        );
    }

    #[test]
    fn xiph_laced_zero_length_frame_is_rejected() {
        // Xiph lacing, two frames, first lace size 0, then one byte of data
        // for the second frame: the first frame is empty.
        let block = [0x81u8, 0x00, 0x00, 0x02, 0x01, 0x00, 0xAA];
        let err = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[test]
    fn declared_frame_count_cannot_exceed_the_file_length() {
        // Defence-in-depth check, asserted directly because a *well-formed*
        // lacing cannot reach it: every frame carries at least one payload byte
        // (a zero-length lace is rejected above) and at least one size byte, so
        // N frames always need more than N file bytes. `demux` is exercised
        // through a whole small file whose one block claims 256 frames.
        //
        // Hand-assembled EBML so the test does not depend on the test-only
        // builder in `tests/webm_demux.rs`:
        //   EBML header | Segment { Cluster { Timecode, SimpleBlock } }
        let block_payload = [
            0x81u8, 0x00, 0x00, 0x04, 0xFF, 0xAA, 0xBB, // track,ts,flags=fixed,count=255,+2
        ];
        let mut cluster_body = alloc::vec![0xE7, 0x81, 0x00]; // Timecode 0
        cluster_body.extend_from_slice(&[0xA3, 0x87]); // SimpleBlock, size 7
        cluster_body.extend_from_slice(&block_payload);
        let mut segment_body = alloc::vec![0x16, 0x54, 0xAE, 0x6B, 0x80]; // Tracks, size 0
        segment_body.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75, 0x80 | cluster_body.len() as u8]);
        segment_body.extend_from_slice(&cluster_body);
        let mut file = alloc::vec![0x1A, 0x45, 0xDF, 0xA3, 0x80]; // EBML, size 0
        file.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0x80 | segment_body.len() as u8]);
        file.extend_from_slice(&segment_body);

        let mut demux = WebmDemux::new();
        let result = demux.demux(&file);
        assert!(
            result.is_err(),
            "a block declaring more frames than the file has bytes must not demux              (got {:?} tracks)",
            result.map(|m| m.tracks.len())
        );

        // The same block, but with a count the file can actually carry, parses
        // — so the rejection above is about the malformed count, not about the
        // block being unparseable at all.
        let frames = demux_frame_count(&[0x81u8, 0x00, 0x00, 0x02, 0x01, 0x00, 0xAA]);
        assert_eq!(frames, None, "a zero-length lace is rejected");
        let frames = demux_frame_count(&[0x81u8, 0x00, 0x00, 0x02, 0x01, 0x01, 0xAA, 0xBB]);
        assert_eq!(frames, Some(2), "a two-frame Xiph lace parses");
    }

    /// Number of frames `parse_block` recovers from `block`, or `None` if it is
    /// rejected.
    fn demux_frame_count(block: &[u8]) -> Option<usize> {
        parse_block(block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true)
            .ok()
            .map(|b| b.frames.len())
    }

    #[test]
    fn a_full_lace_count_is_accepted_and_the_cap_is_the_format_maximum() {
        // The count byte is `FrameCount - 1` (RFC 9559 §12), so the format's own
        // ceiling is 256 and `MAX_LACED_FRAMES` must be exactly that — a smaller
        // value would reject a conformant block, a larger one would not bound
        // anything.
        assert_eq!(
            MAX_LACED_FRAMES,
            usize::from(u8::MAX) + 1,
            "the cap is the format's own maximum frame count"
        );

        // A block using the whole count with 256 one-byte frames is conformant
        // and must parse, producing exactly that many frames.
        let mut block = alloc::vec![0x81u8, 0x00, 0x00, 0x04, 0xFF];
        block.extend(core::iter::repeat_n(0xAAu8, 256));
        let parsed = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).expect("parse");
        assert_eq!(parsed.frames.len(), MAX_LACED_FRAMES);
        assert_eq!(parsed.declared_frames, MAX_LACED_FRAMES);
        assert!(parsed.frames.iter().all(|f| f.len() == 1));
    }

    #[test]
    fn laced_frame_size_past_the_payload_is_rejected() {
        // Two frames, first lace size 0x05 but only 2 bytes follow.
        let block = [0x81u8, 0x00, 0x00, 0x02, 0x01, 0x05, 0xAA, 0xBB];
        let err = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    /// r04-W43: an AVC `CodecPrivate` declaring 2-byte NAL lengths must have its
    /// block payloads rewritten to 4-byte prefixes on the way into the IR, and
    /// **the emitted `avcC` must declare that same 4-byte size** — otherwise a
    /// fMP4/CMAF/TS mux of the normalised IR writes an `avcC` that promises
    /// 2-byte lengths over 4-byte samples.
    ///
    /// The crate's own `MkvMux` writes `CodecPrivate` and block payloads
    /// verbatim, so a `Media` holding a 2-byte `avcC` and 2-byte-prefixed
    /// samples round-trips through mux → demux; the NAL bodies must survive
    /// byte-for-byte under a 4-byte prefix, and `length_size_minus_one` must be
    /// 3. Then muxing the resulting IR to an init segment + fragment and
    /// demuxing *that* must yield the same NALs (end-to-end, not just an
    /// in-memory field check).
    #[test]
    fn webm_avc_two_byte_nal_lengths_normalised_to_four() {
        use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
        use crate::ir::{FragmentTrackData, Sample, SampleFlags};
        use crate::mkv_mux::MkvMux;
        use crate::nalu_types::{AvcPps, AvcSps};
        use crate::pipeline::{CodecConfig, TrackSpec, build_init_segment, build_media_segment};
        use broadcast_common::Package;

        let record = AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 66,
            profile_compatibility: 0,
            level_indication: 0x1E,
            // 1 → 2-byte NAL length prefixes.
            length_size_minus_one: 1,
            sps: alloc::vec![AvcSps(alloc::vec![0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40])],
            pps: alloc::vec![AvcPps(alloc::vec![0x68, 0xCE, 0x3C, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: alloc::vec![],
        };
        let config = CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 640,
            height: 360,
        };
        // Two NALs, each with a 2-byte length prefix, in *one* access unit.
        let sample = Sample {
            data: alloc::vec![0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x01, 0x41].into(),
            dts: Some(0),
            pts: Some(0),
            duration: Some(1000),
            flags: SampleFlags::new(true),
            provenance: None,
        };
        let media = Media::new(
            alloc::vec![Track::new_at(
                TrackSpec::new(1, IR_TIMESCALE, config),
                alloc::vec![sample],
                0,
            )],
            IR_TIMESCALE,
        );
        let muxed = MkvMux::new().package(&media).expect("mkv package");
        let demuxed = WebmDemux::new()
            .unpackage(&muxed)
            .expect("WebM demux of the 2-byte-length file");
        let track = &demuxed.tracks[0];

        const EXPECTED: [u8; 14] = [
            0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, // NAL A, 4-byte prefix
            0x00, 0x00, 0x00, 0x01, 0x41, // NAL B, 4-byte prefix
        ];
        assert_eq!(
            track.samples[0].data.as_ref(),
            EXPECTED,
            "2-byte NAL lengths must be rewritten to 4-byte on the demux edge"
        );
        let CodecConfig::Avc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Avc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the emitted avcC must declare the 4-byte length size of the samples"
        );

        // End-to-end: mux the IR to CMAF, then demux it back. The re-parsed
        // avcC declares 4-byte lengths and the sample's NALs are unchanged.
        let init = build_init_segment(core::slice::from_ref(&track.spec), IR_TIMESCALE)
            .expect("init segment");
        let frag = build_media_segment(1, &[FragmentTrackData::new(1, 0, &track.samples)])
            .expect("media segment");
        let mut both = init;
        both.extend_from_slice(&frag);
        let reparsed = crate::media::Fmp4Demux::new()
            .unpackage(&both)
            .expect("re-demux of the muxed normalised IR");
        assert_eq!(
            reparsed.tracks[0].samples[0].data.as_ref(),
            EXPECTED,
            "the 4-byte-prefixed NALs must survive a CMAF mux/demux round-trip"
        );
        let CodecConfig::Avc { config, .. } = &reparsed.tracks[0].spec.config else {
            panic!("expected CodecConfig::Avc");
        };
        assert_eq!(
            config.config.length_size_minus_one,
            NAL_LENGTH_SIZE_MINUS_ONE
        );
    }

    /// The same for the HEVC (`hvcC`) path: a 2-byte `lengthSizeMinusOne` must
    /// be normalised to 4 on the samples *and* on the emitted record.
    #[test]
    fn webm_hevc_two_byte_nal_lengths_normalised_to_four() {
        use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
        use crate::ir::{FragmentTrackData, Sample, SampleFlags};
        use crate::mkv_mux::MkvMux;
        use crate::nalu_types::{HevcNalArray, HevcNalUnit};
        use crate::pipeline::{CodecConfig, TrackSpec, build_init_segment, build_media_segment};
        use broadcast_common::Package;

        let record = HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0,
            general_constraint_indicator_flags: 0,
            general_level_idc: 93,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: false,
            // 1 → 2-byte NAL length prefixes.
            length_size_minus_one: 1,
            arrays: alloc::vec![HevcNalArray {
                array_completeness: true,
                nal_unit_type: 32,
                nalus: alloc::vec![HevcNalUnit(alloc::vec![0x40, 0x01, 0x0C, 0x01, 0xFF])],
            }],
        };
        let config = CodecConfig::Hevc {
            config: HEVCConfigurationBox::new(record),
            width: 640,
            height: 360,
        };
        let sample = Sample {
            data: alloc::vec![0x00, 0x04, 0x26, 0x01, 0xAF, 0x09].into(),
            dts: Some(0),
            pts: Some(0),
            duration: Some(1000),
            flags: SampleFlags::new(true),
            provenance: None,
        };
        let media = Media::new(
            alloc::vec![Track::new_at(
                TrackSpec::new(1, IR_TIMESCALE, config),
                alloc::vec![sample],
                0,
            )],
            IR_TIMESCALE,
        );
        let muxed = MkvMux::new().package(&media).expect("mkv package");
        let demuxed = WebmDemux::new().unpackage(&muxed).expect("WebM demux");
        let track = &demuxed.tracks[0];
        assert_eq!(
            track.samples[0].data.as_ref(),
            [0x00, 0x00, 0x00, 0x04, 0x26, 0x01, 0xAF, 0x09],
            "HEVC 2-byte NAL lengths must be rewritten to 4-byte"
        );
        let CodecConfig::Hevc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Hevc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the emitted hvcC must declare the 4-byte length size"
        );

        // End-to-end: mux the normalised IR to CMAF and demux it back. The
        // re-parsed `hvcC` declares 4-byte lengths and the NAL bytes survive.
        let expected: [u8; 8] = [0x00, 0x00, 0x00, 0x04, 0x26, 0x01, 0xAF, 0x09];
        let init =
            build_init_segment(core::slice::from_ref(&track.spec), IR_TIMESCALE).expect("init");
        let frag = build_media_segment(1, &[FragmentTrackData::new(1, 0, &track.samples)])
            .expect("media segment");
        let mut both = init;
        both.extend_from_slice(&frag);
        let reparsed = crate::media::Fmp4Demux::new()
            .unpackage(&both)
            .expect("re-demux of the muxed normalised HEVC IR");
        assert_eq!(
            reparsed.tracks[0].samples[0].data.as_ref(),
            expected,
            "4-byte-prefixed HEVC NALs must survive a CMAF mux/demux round-trip"
        );
        let CodecConfig::Hevc { config, .. } = &reparsed.tracks[0].spec.config else {
            panic!("expected CodecConfig::Hevc after re-demux");
        };
        assert_eq!(
            config.config.length_size_minus_one,
            NAL_LENGTH_SIZE_MINUS_ONE
        );
    }
}
