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
//!
//! # Scope
//!
//! Only the elements needed to demux VP9-video + Opus-audio WebM are decoded;
//! `SeekHead`, `Cues`, `Tags` and any other master element are skipped by size.
//! Only `V_VP9` (→ [`CodecConfig::Vp9`]) and `A_OPUS` (→ [`CodecConfig::Opus`])
//! CodecIDs build a track; every other CodecID is skipped (never fatal). Lacing
//! is not supported: a laced block is a hard error (VP9/Opus WebM from ffmpeg use
//! no lacing — one frame per block).
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

use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::Unpackage;

use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::opus::OpusSpecificBox;
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
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
/// `ReferenceBlock` (BlockGroup child; present ⇒ the Block references another
/// block ⇒ not a random-access point). Its absence marks the Block a keyframe.
const REFERENCE_BLOCK: u32 = 0xFB;

// --- Matroska CodecIDs (RFC 9559 codec-mapping registry) ---------------------
/// VP9 video CodecID.
const CODEC_V_VP9: &[u8] = b"V_VP9";
/// Opus audio CodecID.
const CODEC_A_OPUS: &[u8] = b"A_OPUS";

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
    /// The coded frame bytes.
    data: Vec<u8>,
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
        while let Some((id, child)) = r.next_element()? {
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
                CLUSTER_TIMESTAMP => cluster_ts = read_uint(child) as i64,
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
/// flags byte, then (no lacing) the single frame. `is_simple_block` selects
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

    if flags & BLOCK_FLAG_LACING_MASK != 0 {
        return Err(Error::InvalidInput(
            "webm block: lacing is not supported (expected one frame per block)",
        ));
    }
    let is_sync = if is_simple_block {
        flags & BLOCK_FLAG_KEYFRAME != 0
    } else {
        false
    };

    // Presentation time in IR ticks (ms): (cluster_ts + rel_ts) ticks × scale(ns)
    // → ns → ms.  ns = raw_ticks × timestamp_scale_ns; ms = ns / (NS_PER_SECOND / IR_TIMESCALE).
    let raw_ticks = cluster_ts + rel_ts;
    let ns = raw_ticks.saturating_mul(timestamp_scale_ns as i64);
    let ns_per_ir_tick = (NS_PER_SECOND / IR_TIMESCALE as u64) as i64;
    let pts_ticks = ns / ns_per_ir_tick;

    Ok(RawBlock {
        track_number,
        pts_ticks,
        is_sync,
        data: data[off..].to_vec(),
    })
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
    blocks: Vec<RawBlock>,
) -> Result<Media> {
    let _ = timestamp_scale_ns;
    let mut out_tracks: Vec<Track> = Vec::new();
    let mut track_id: u32 = 1;

    for info in &tracks {
        let Some(config) = codec_config_for(info)? else {
            continue; // Unsupported CodecID: skip, never fatal.
        };

        // Gather this track's blocks in file (decode) order.
        let mut samples: Vec<Sample> = Vec::new();
        let mut pts: Vec<i64> = Vec::new();
        let mut sync: Vec<bool> = Vec::new();
        let mut payloads: Vec<Vec<u8>> = Vec::new();
        for b in &blocks {
            if b.track_number == info.track_number {
                pts.push(b.pts_ticks);
                sync.push(b.is_sync);
                payloads.push(b.data.clone());
            }
        }
        if payloads.is_empty() {
            continue;
        }

        // Per-sample duration = delta to next block's PTS; last reuses prior
        // delta (or DefaultDuration in IR ticks when a single block).
        let default_dur_ir = info.default_duration_ns / (NS_PER_SECOND / IR_TIMESCALE as u64);
        let n = payloads.len();
        for i in 0..n {
            let duration = if i + 1 < n {
                (pts[i + 1] - pts[i]).max(0) as u32
            } else if n >= 2 {
                (pts[i] - pts[i - 1]).max(0) as u32
            } else {
                default_dur_ir as u32
            };
            samples.push(Sample {
                data: core::mem::take(&mut payloads[i]),
                duration,
                is_sync: sync[i],
                composition_offset: 0,
            });
        }

        out_tracks.push(Track::new(
            TrackSpec {
                track_id,
                timescale: IR_TIMESCALE,
                config,
            },
            samples,
        ));
        track_id += 1;
    }

    Ok(Media::new(out_tracks, IR_TIMESCALE))
}

/// Map a [`TrackInfo`] to a [`CodecConfig`], or `None` for an unsupported CodecID.
fn codec_config_for(info: &TrackInfo) -> Result<Option<CodecConfig>> {
    if info.track_type == TRACK_TYPE_VIDEO && info.codec_id == CODEC_V_VP9 {
        Ok(Some(vp9_config(info)))
    } else if info.track_type == TRACK_TYPE_AUDIO && info.codec_id == CODEC_A_OPUS {
        Ok(Some(opus_config(info)?))
    } else {
        Ok(None)
    }
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
    /// end of the enclosing buffer (used for live Segment/Cluster).
    fn next_element(&mut self) -> Result<Option<(u32, &'a [u8])>> {
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
            self.buf.len()
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
    fn lacing_rejected() {
        // track-number VINT (0x81), rel-ts int16 (0,0), flags with Xiph lacing (0x02).
        let block = [0x81u8, 0x00, 0x00, 0x02, 0xAA];
        let err = parse_block(&block, 0, DEFAULT_TIMESTAMP_SCALE_NS, true).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }
}
