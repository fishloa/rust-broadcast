//! The streaming Matroska/WebM demux stage over
//! [`broadcast_matroska::reader::Reader`].
//!
//! [`MatroskaDemux`] is the one Matroska demuxer: it answers the reader's
//! byte demands through [`Stage::demand`] and emits the crate's existing
//! [`DemuxEvent`] vocabulary, so a segment consumer keeps one stage per
//! representation and a file consumer reads through bounded byte ranges. It
//! never holds a whole file: a seekable input delivers exactly the ranges the
//! reader asks for, a sequential input holds one bounded unit at a time, and a
//! Block's frames are fetched by range and dropped once their samples are
//! emitted.
//!
//! The reader is always created in [`Mode::Permissive`]: a fault it tolerates
//! becomes [`DemuxEvent::InputDegraded`], never a silent skip. A fault it
//! cannot tolerate is the stage's error, after which the stage is poisoned.
//!
//! Sample timing and conditioning are exactly the whole-input demuxer's this
//! replaces: Matroska times a Block (RFC 9559 §12), so a laced Block's frames
//! are consecutive from the Block's own time at the track's frame cadence
//! (`DefaultDuration`, else the interval to the next Block divided among the
//! earlier Block's frames). Every laced frame becomes its own sample. `dts`
//! equals `pts` (WebM's mapped codecs carry no reordering), and the codec
//! mapping (CodecID to [`CodecConfig`], including AVC/HEVC NAL length
//! normalisation) is the whole-input demuxer's, moved here unchanged.
//!
//! # Spec citations
//!
//! - **EBML framing**: RFC 8794 §4-§6
//!   (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`).
//! - **Matroska Segment/Info/Tracks/Cluster/Block/Cues**: RFC 9559 §5, §6,
//!   §11 (timestamps), §12 (Block structure), §20 (Cues)
//!   (`private/specs-verified/ietf/specs/rfc9559_matroska.txt`), transcribed
//!   for this crate at
//!   `private/specs-verified/mpeg/transmux/webm/ebml-matroska.md`.
//! - **VP8** key-frame header (dimensions): RFC 6386 §9.1 / §19.1.
//! - **Vorbis** `CodecPrivate` (Xiph-laced three headers) + Identification
//!   header: the Vorbis I specification §4.2.2
//!   (`private/specs-verified/mpeg/transmux/codec/vp8-vorbis-webm.md`).

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::Stage;
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::len::need_in;
use broadcast_common::stage::{Demand, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, rescale_u64};
use broadcast_common::wire::be_u64;
use broadcast_matroska::Limits;
use broadcast_matroska::reader::{
    BlockStep, ByteRange, Mode, Reader, SegmentInfo, Source, Step, TrackInfo, Warning,
};
use bytes::{Bytes, BytesMut};

use crate::TrackId;
use crate::annexb::{NAL_LENGTH_SIZE_MINUS_ONE, normalise_nal_length_size};
use crate::error::{Error, Result};
use crate::ir::{
    CodecConfig, DataIdentity, DemuxEvent, EventProvenance, InputDegradation, Sample, SampleFlags,
    SampleTiming, TrackSpec,
};
use crate::rtp_sdp::aac_config_from_asc_bytes;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use isobmff::opus::{ChannelMappingTable, OpusSpecificBox};
use isobmff::vp9::Vp9ConfigurationBox;

/// The IR timescale (ticks per second) this demuxer emits: milliseconds.
pub const IR_TIMESCALE: Timescale = broadcast_common::timescale!(1000);

/// Nanoseconds as a [`Timescale`] (a `DefaultDuration` is a count of ns).
const NS_TIMESCALE: Timescale = broadcast_common::timescale!(1_000_000_000);

/// Nanoseconds in one IR tick.
const IR_TICK_NS: i128 = 1_000_000;

/// Matroska CodecIDs this demuxer maps (RFC 9559 codec-mapping registry).
const CODEC_V_VP9: &str = "V_VP9";
/// VP8 video CodecID.
const CODEC_V_VP8: &str = "V_VP8";
/// H.264/AVC video CodecID; `CodecPrivate` is the raw
/// `AVCDecoderConfigurationRecord` (ISO/IEC 14496-15:2017 §5.3.3).
const CODEC_V_AVC: &str = "V_MPEG4/ISO/AVC";
/// H.265/HEVC video CodecID; `CodecPrivate` is the raw
/// `HEVCDecoderConfigurationRecord` (ISO/IEC 14496-15:2017 §8.3.3.1).
const CODEC_V_HEVC: &str = "V_MPEGH/ISO/HEVC";
/// Opus audio CodecID.
const CODEC_A_OPUS: &str = "A_OPUS";
/// Vorbis audio CodecID.
const CODEC_A_VORBIS: &str = "A_VORBIS";
/// AAC audio CodecID; `CodecPrivate` is the raw `AudioSpecificConfig`
/// (ISO/IEC 14496-3 §1.6.2.1).
const CODEC_A_AAC: &str = "A_AAC";

/// `TrackType` value for a video track (RFC 9559 §27).
const TRACK_TYPE_VIDEO: u64 = 1;
/// `TrackType` value for an audio track.
const TRACK_TYPE_AUDIO: u64 = 2;

/// OpusHead identification-header magic (RFC 7845 §5.1).
const OPUS_HEAD_MAGIC: &[u8; 8] = b"OpusHead";
/// Minimum OpusHead length: magic(8) + version(1) + channels(1) + pre-skip(2)
/// + input-rate(4) + output-gain(2) + mapping-family(1).
const OPUS_HEAD_MIN_LEN: usize = 19;
/// Offset of `OpusHead.version`.
const OPUS_HEAD_VERSION_OFFSET: usize = OPUS_HEAD_MAGIC.len();
/// Offset of `OpusHead.channel_count`.
const OPUS_HEAD_CHANNELS_OFFSET: usize = OPUS_HEAD_VERSION_OFFSET + 1;
/// Offset of the little-endian `OpusHead.pre_skip`.
const OPUS_HEAD_PRE_SKIP_OFFSET: usize = OPUS_HEAD_CHANNELS_OFFSET + 1;
/// Offset of the little-endian `OpusHead.input_sample_rate`.
const OPUS_HEAD_INPUT_RATE_OFFSET: usize = OPUS_HEAD_PRE_SKIP_OFFSET + 2;
/// Offset of the little-endian `OpusHead.output_gain`.
const OPUS_HEAD_OUTPUT_GAIN_OFFSET: usize = OPUS_HEAD_INPUT_RATE_OFFSET + 4;
/// Offset of `OpusHead.channel_mapping_family`.
const OPUS_HEAD_MAPPING_FAMILY_OFFSET: usize = OPUS_HEAD_OUTPUT_GAIN_OFFSET + 2;
/// Offset of the channel-mapping table's `stream_count` (RFC 7845 §5.1.1).
const OPUS_HEAD_STREAM_COUNT_OFFSET: usize = OPUS_HEAD_MIN_LEN;
/// Offset of the channel-mapping table's `coupled_count`.
const OPUS_HEAD_COUPLED_COUNT_OFFSET: usize = OPUS_HEAD_STREAM_COUNT_OFFSET + 1;
/// Offset of the channel-mapping table's `channel_mapping` array.
const OPUS_HEAD_CHANNEL_MAPPING_OFFSET: usize = OPUS_HEAD_COUPLED_COUNT_OFFSET + 1;
/// Opus playback rate is always 48 kHz (RFC 7845 §5.1).
const OPUS_OUTPUT_SAMPLE_RATE: u32 = 48_000;
/// Audio sample size in bits carried in the sample entry (convention: 16).
const AUDIO_SAMPLE_SIZE: u16 = 16;

/// VP8 uncompressed frame tag length (RFC 6386 §9.1).
const VP8_FRAME_TAG_LEN: usize = 3;
/// VP8 key-frame start code (RFC 6386 §9.1).
const VP8_START_CODE: [u8; 3] = [0x9D, 0x01, 0x2A];
/// `key_frame` bit: `0` marks a key frame (RFC 6386 §9.1).
const VP8_KEYFRAME_TAG_BIT: u8 = 0x01;
/// The width/height low-14-bit mask (RFC 6386 §9.1).
const VP8_DIMENSION_MASK: u16 = 0x3FFF;
/// Bytes of one VP8 dimension word.
const VP8_DIMENSION_LEN: usize = core::mem::size_of::<u16>();
/// Minimum VP8 key-frame header length.
const VP8_KEYFRAME_HEADER_LEN: usize =
    VP8_FRAME_TAG_LEN + VP8_START_CODE.len() + 2 * VP8_DIMENSION_LEN;

/// Xiph-lacing header count byte for the three Vorbis headers: `numPackets - 1`.
const VORBIS_LACE_COUNT: u8 = 2;
/// The Vorbis Identification-header packet type (Vorbis I §4.2.1).
const VORBIS_ID_HEADER_TYPE: u8 = 0x01;
/// The six-byte "vorbis" signature following every header's type byte.
const VORBIS_SIGNATURE: &[u8; 6] = b"vorbis";
/// Offset of `audio_channels` within the Identification header.
const VORBIS_ID_CHANNELS_OFFSET: usize = 1 + 6 + 4;
/// Offset of the little-endian `audio_sample_rate`.
const VORBIS_ID_SAMPLE_RATE_OFFSET: usize = VORBIS_ID_CHANNELS_OFFSET + 1;
/// Bytes of the little-endian `audio_sample_rate`.
const VORBIS_ID_SAMPLE_RATE_LEN: usize = core::mem::size_of::<u32>();
/// Minimum Identification-header length to read channels and sample rate.
const VORBIS_ID_MIN_LEN: usize = VORBIS_ID_SAMPLE_RATE_OFFSET + VORBIS_ID_SAMPLE_RATE_LEN;
/// Laced header lengths stored explicitly (all but the last).
const VORBIS_LACING_LEN_FIELDS: usize = VORBIS_LACE_COUNT as usize;
/// A Xiph lacing length byte at this value continues the run (Vorbis I §4.2.2).
const XIPH_LACE_CONTINUATION: u8 = 0xFF;

/// VPCodecConfigurationBox version (`FullBox` v1).
const VPCC_VERSION: u8 = 1;
/// VP9 profile 0 (8-bit 4:2:0), the default when not derivable.
const VP9_PROFILE_0: u8 = 0;
/// VP9 level "unspecified/undefined".
const VP9_LEVEL_UNSPECIFIED: u8 = 0;
/// Default VP9 bit depth.
const VP9_BIT_DEPTH_8: u8 = 8;
/// `chroma_subsampling` = 1 (4:2:0 co-located with luma).
const VP9_CHROMA_420: u8 = 1;
/// CICP `colour_primaries` = 2 (unspecified).
const CICP_UNSPECIFIED: u8 = 2;

/// Fewest blocks that give a sample duration from a neighbouring timestamp.
const MIN_BLOCKS_FOR_DELTA: usize = 2;

/// Which way the stage reaches its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceMode {
    /// Arbitrary byte ranges are available (a file).
    Seekable,
    /// The input only moves forward (a pipe or socket).
    Sequential,
}

/// One unit of input handed to [`MatroskaDemux`].
#[non_exhaustive]
#[derive(Debug)]
pub enum MatroskaInput<'a> {
    /// The bytes at `offset`, answering a [`Demand::at`] request. `bytes` may
    /// be shorter than requested only when the input ends there.
    At {
        /// Absolute input offset of the first byte.
        offset: u64,
        /// The bytes themselves.
        bytes: Bytes,
    },
    /// The next sequential bytes (never used by a seekable stage).
    Next(&'a [u8]),
    /// No more input will arrive.
    Eof,
}

/// Configuration for [`MatroskaDemux`].
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct MatroskaDemuxConfig {
    limits: Limits,
    total_len: Option<u64>,
}

impl MatroskaDemuxConfig {
    /// Set the hostile-input caps.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Set the total input length, when the caller knows it.
    #[must_use]
    pub fn with_total_len(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// The hostile-input caps.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The total input length, when the caller knows it.
    #[must_use]
    pub fn total_len(&self) -> Option<u64> {
        self.total_len
    }
}

/// One owned reader step, so the reader is never borrowed across a demand.
#[derive(Debug)]
enum OwnedStep {
    Need(ByteRange),
    Info(SegmentInfo),
    Tracks(Vec<TrackInfo>),
    Block(BlockStep),
    Warning(Warning),
    Done,
    Ignored,
}

/// One Block of one track, held until its per-frame timeline can be laid out.
#[derive(Debug)]
struct HeldBlock {
    pts: i64,
    keyframe: bool,
    frames: Vec<Bytes>,
}

impl HeldBlock {
    fn new(pts: i64, keyframe: bool) -> Self {
        Self {
            pts,
            keyframe,
            frames: Vec::new(),
        }
    }
}

/// One track's mapping and timing state.
struct TrackState {
    id: Option<TrackId>,
    number: u64,
    info: TrackInfo,
    requires_frame_config: bool,
    config: Option<(CodecConfig, Option<usize>)>,
    spec: Option<TrackSpec>,
    nal_length_size: Option<usize>,
    announced: bool,
    emitted: bool,
    abandoned: bool,
    nominal: Option<u64>,
    blocks: VecDeque<HeldBlock>,
    held_bytes: usize,
    total_frames: u64,
    last_pts: Option<i64>,
}

impl TrackState {
    fn is_resolved(&self) -> bool {
        (self.announced || self.abandoned) && self.emitted
    }

    fn has_frames(&self) -> bool {
        self.total_frames > 0
    }
}

impl TrackState {
    /// Score a newly arrived frame into this track's held Blocks and emit every
    /// sample whose timeline is now known.
    fn push_frame(
        &mut self,
        block: &BlockStep,
        data: Bytes,
        byte_cap: u64,
        out: &mut VecDeque<DemuxEvent>,
    ) -> Result<()> {
        if block.lace_index == 0 {
            let pts = block_pts(block)?;
            self.blocks.push_back(HeldBlock::new(pts, block.keyframe));
        }
        let Some(held) = self.blocks.back_mut() else {
            return Err(Error::InvalidInput(
                "matroska: a block frame arrived with no block open",
            ));
        };
        self.held_bytes = self.held_bytes.saturating_add(data.len());
        self.total_frames = self.total_frames.saturating_add(1);
        held.frames.push(data);
        self.drain(byte_cap, true, out)
    }

    /// Emit every held Block whose successor (or end of track) is known.
    fn drain(
        &mut self,
        byte_cap: u64,
        streaming: bool,
        out: &mut VecDeque<DemuxEvent>,
    ) -> Result<()> {
        if !self.emitted {
            return Ok(());
        }
        if self.nominal.is_none() {
            self.nominal = derive_nominal(&self.blocks);
        }
        if self.nominal.is_none() && u64::try_from(self.held_bytes).unwrap_or(u64::MAX) > byte_cap {
            self.nominal = Some(0);
        }
        while self.blocks.len() >= MIN_BLOCKS_FOR_DELTA {
            let Some(nominal) = self.nominal else {
                break;
            };
            let Some(block) = self.blocks.pop_front() else {
                break;
            };
            let successor = self.blocks.front().map(|next| next.pts);
            self.emit_block(block, nominal, successor, out)?;
        }
        if !streaming {
            let nominal = self.nominal.unwrap_or(0);
            while let Some(block) = self.blocks.pop_front() {
                let successor = self.blocks.front().map(|next| next.pts);
                self.emit_block(block, nominal, successor, out)?;
            }
        }
        Ok(())
    }

    /// Emit one Block's frames as samples.
    fn emit_block(
        &mut self,
        block: HeldBlock,
        nominal: u64,
        successor: Option<i64>,
        out: &mut VecDeque<DemuxEvent>,
    ) -> Result<()> {
        let Some(id) = self.id else {
            return Ok(());
        };
        let step = nominal.max(1);
        let count = block.frames.len();
        let keyframe = block.keyframe;
        for (index, data) in block.frames.into_iter().enumerate() {
            self.held_bytes = self.held_bytes.saturating_sub(data.len());
            let pts = block.pts.saturating_add(
                i64::try_from(index)
                    .unwrap_or(i64::MAX)
                    .saturating_mul(i64::try_from(step).unwrap_or(i64::MAX)),
            );
            let delta: i64 = if index + 1 < count {
                i64::try_from(step).unwrap_or(i64::MAX)
            } else if let Some(next) = successor {
                next.saturating_sub(pts).max(0)
            } else if let Some(previous) = self.last_pts {
                pts.saturating_sub(previous).max(0)
            } else {
                i64::try_from(nominal).unwrap_or(i64::MAX)
            };
            let duration = u32::try_from(delta.max(0)).map_err(|_| {
                Error::InvalidInput("matroska: sample duration does not fit the IR")
            })?;
            let data = match self.nal_length_size {
                Some(size) => Bytes::from(normalise_nal_length_size(&data, size)?),
                None => data,
            };
            self.last_pts = Some(pts);
            out.push_back(DemuxEvent::sample(
                id,
                Sample {
                    data,
                    timing: Some(SampleTiming::timed_with_duration(pts, pts, duration)),
                    flags: SampleFlags::new(keyframe),
                    provenance: None,
                    display: None,
                    protection: None,
                },
            ));
        }
        Ok(())
    }
}

/// The streaming Matroska demux stage.
pub struct MatroskaDemux {
    config: MatroskaDemuxConfig,
    source: SourceMode,
    reader: Reader,
    events: VecDeque<DemuxEvent>,
    tracks: Vec<TrackState>,
    need: Option<ByteRange>,
    frame: Option<BlockStep>,
    at: Option<(u64, Bytes)>,
    buf: BytesMut,
    buf_start: u64,
    retain: u64,
    eof: bool,
    done: bool,
    finished: bool,
    flushed: bool,
    resolved: bool,
    next_track_id: u32,
    poisoned: Option<Error>,
}

impl MatroskaDemux {
    /// A stage over a source that can serve any byte range (a file).
    #[must_use]
    pub fn seekable(config: MatroskaDemuxConfig) -> Self {
        let source = Source::Seekable {
            len: config.total_len.unwrap_or(u64::MAX),
        };
        Self::create(config, SourceMode::Seekable, source)
    }

    /// A stage over a source that only moves forward (a pipe or socket).
    #[must_use]
    pub fn sequential(config: MatroskaDemuxConfig) -> Self {
        Self::create(config, SourceMode::Sequential, Source::ForwardOnly)
    }

    fn create(config: MatroskaDemuxConfig, mode: SourceMode, source: Source) -> Self {
        let reader = Reader::new(source, Mode::Permissive, config.limits);
        let retain = config
            .limits
            .max_block_header_bytes()
            .max(broadcast_matroska::reader::MAX_ELEMENT_HEADER_BYTES);
        let mut demux = Self {
            config,
            source: mode,
            reader,
            events: VecDeque::new(),
            tracks: Vec::new(),
            need: None,
            frame: None,
            at: None,
            buf: BytesMut::new(),
            buf_start: 0,
            retain,
            eof: false,
            done: false,
            finished: false,
            flushed: false,
            resolved: false,
            next_track_id: 1,
            poisoned: None,
        };
        if let Err(err) = demux.pump() {
            demux.poisoned = Some(err);
        }
        demux
    }

    /// Every mapped track's spec, available as soon as the `Tracks` element
    /// has been read (no `finish`).
    pub fn init_tracks(&self) -> impl Iterator<Item = &TrackSpec> + '_ {
        self.tracks.iter().filter_map(|track| track.spec.as_ref())
    }

    /// Bytes the sequential buffer currently holds, and the capacity it keeps
    /// allocated for more.
    #[must_use]
    pub fn buffered(&self) -> (usize, usize) {
        (self.buf.len(), self.buf.capacity())
    }

    fn pump(&mut self) -> Result<()> {
        if let Some(err) = &self.poisoned {
            return Err(err.clone());
        }
        let result = self.pump_inner();
        if let Err(err) = &result {
            self.poisoned = Some(err.clone());
        }
        result
    }

    fn pump_inner(&mut self) -> Result<()> {
        loop {
            if let Some(block) = self.frame {
                if block.frame.len == 0 {
                    self.on_frame(block, Bytes::new())?;
                    self.frame = None;
                    continue;
                }
                if self.take_frame(block)? {
                    self.frame = None;
                    continue;
                }
                return Ok(());
            }
            if let Some(range) = self.need {
                if range.len == 0 {
                    self.reader.supply(range.offset, &[])?;
                    self.need = None;
                    continue;
                }
                if self.serve_need(range)? {
                    self.need = None;
                    continue;
                }
                return Ok(());
            }
            if self.done {
                self.finish_flush()?;
                return Ok(());
            }
            match self.next_owned_step()? {
                OwnedStep::Need(range) => self.need = Some(range),
                OwnedStep::Info(info) => self.on_info(info),
                OwnedStep::Tracks(tracks) => self.on_tracks(tracks)?,
                OwnedStep::Block(block) => self.frame = Some(block),
                OwnedStep::Warning(warning) => self.push_warning(warning),
                OwnedStep::Done => self.done = true,
                OwnedStep::Ignored => {}
            }
        }
    }

    fn next_owned_step(&mut self) -> Result<OwnedStep> {
        Ok(match self.reader.step()? {
            Step::Need(range) => OwnedStep::Need(range),
            Step::Info { info, .. } => OwnedStep::Info(info),
            Step::Tracks { tracks, .. } => OwnedStep::Tracks(tracks.to_vec()),
            Step::Block(block) => OwnedStep::Block(block),
            Step::Warning(warning) => OwnedStep::Warning(warning),
            Step::Done => OwnedStep::Done,
            _ => OwnedStep::Ignored,
        })
    }

    fn on_info(&mut self, _info: SegmentInfo) {}

    fn on_tracks(&mut self, infos: Vec<TrackInfo>) -> Result<()> {
        self.tracks.clear();
        for info in infos {
            let requires_frame_config =
                info.track_type_value() == TRACK_TYPE_VIDEO && info.codec_id() == CODEC_V_VP8;
            let default_duration_ir = info.default_duration_ns().map_or(0, ns_to_ir_ticks);
            self.tracks.push(TrackState {
                id: None,
                number: info.track_number(),
                info,
                requires_frame_config,
                config: None,
                spec: None,
                nal_length_size: None,
                announced: false,
                emitted: false,
                abandoned: false,
                nominal: (default_duration_ir > 0).then_some(default_duration_ir),
                blocks: VecDeque::new(),
                held_bytes: 0,
                total_frames: 0,
                last_pts: None,
            });
        }
        for index in 0..self.tracks.len() {
            if !self.tracks[index].requires_frame_config {
                let (config, nal_length_size) =
                    match codec_config_for(&self.tracks[index].info, &[])? {
                        Some(mapped) => mapped,
                        None => (data_config(&self.tracks[index].info), None),
                    };
                self.announce(index, config, nal_length_size)?;
            }
        }
        self.flush_announcements();
        self.maybe_resolve();
        Ok(())
    }

    fn announce(
        &mut self,
        index: usize,
        config: CodecConfig,
        nal_length_size: Option<usize>,
    ) -> Result<()> {
        let track = &mut self.tracks[index];
        track.nal_length_size = nal_length_size;
        track.config = Some((config, nal_length_size));
        track.announced = true;
        Ok(())
    }

    /// Emit the queued [`DemuxEvent::TrackAdded`] events in track order, so a
    /// deferred codec config (VP8's in-band dimensions) never reorders the
    /// track set.
    fn flush_announcements(&mut self) {
        for index in 0..self.tracks.len() {
            if self.tracks[index].emitted {
                continue;
            }
            if self.tracks[index].abandoned {
                self.tracks[index].emitted = true;
                continue;
            }
            if !self.tracks[index].announced || !self.tracks[index].has_frames() {
                break;
            }
            let track = &mut self.tracks[index];
            let Some((config, nal_length_size)) = track.config.take() else {
                break;
            };
            let id = TrackId::new(self.next_track_id);
            self.next_track_id = self.next_track_id.saturating_add(1);
            let spec = TrackSpec::new(id, IR_TIMESCALE, config);
            track.id = Some(id);
            track.nal_length_size = nal_length_size;
            track.spec = Some(spec.clone());
            track.emitted = true;
            self.events.push_back(DemuxEvent::TrackAdded(spec));
        }
    }

    fn maybe_resolve(&mut self) {
        if self.resolved || !self.tracks.iter().all(TrackState::is_resolved) {
            return;
        }
        self.resolved = true;
        self.events
            .push_back(DemuxEvent::TracksResolved { generation: 0 });
    }

    fn on_frame(&mut self, block: BlockStep, data: Bytes) -> Result<()> {
        let Some(index) = self
            .tracks
            .iter()
            .position(|track| track.number == block.track)
        else {
            return Ok(());
        };
        if !self.tracks[index].announced
            && self.tracks[index].requires_frame_config
            && block.keyframe
        {
            let (config, nal_length_size) = match codec_config_for(&self.tracks[index].info, &data)?
            {
                Some(mapped) => mapped,
                None => (data_config(&self.tracks[index].info), None),
            };
            self.announce(index, config, nal_length_size)?;
            self.flush_announcements();
        }
        let cap = self.config.limits.max_element_bytes();
        self.tracks[index].push_frame(&block, data, cap, &mut self.events)?;
        self.flush_announcements();
        self.tracks[index].drain(cap, true, &mut self.events)?;
        self.maybe_resolve();
        Ok(())
    }

    fn finish_flush(&mut self) -> Result<()> {
        if self.flushed {
            return Ok(());
        }
        self.flushed = true;
        for index in 0..self.tracks.len() {
            if self.tracks[index].emitted {
                continue;
            }
            if !self.tracks[index].has_frames() {
                self.tracks[index].abandoned = true;
                continue;
            }
            if self.tracks[index].announced {
                continue;
            }
            let first = self.tracks[index]
                .blocks
                .iter()
                .find(|block| block.keyframe)
                .or_else(|| self.tracks[index].blocks.front())
                .and_then(|block| block.frames.first())
                .cloned();
            let (config, nal_length_size) = match codec_config_for(
                &self.tracks[index].info,
                first.as_deref().unwrap_or(&[]),
            )? {
                Some(mapped) => mapped,
                None => (data_config(&self.tracks[index].info), None),
            };
            self.announce(index, config, nal_length_size)?;
        }
        self.flush_announcements();
        let cap = self.config.limits.max_element_bytes();
        for index in 0..self.tracks.len() {
            self.tracks[index].drain(cap, false, &mut self.events)?;
        }
        self.maybe_resolve();
        Ok(())
    }
}

impl MatroskaDemux {
    fn covered(&self, range: ByteRange) -> Option<u64> {
        if range.offset < self.buf_start {
            return None;
        }
        let rel = range.offset - self.buf_start;
        let available = u64::try_from(self.buf.len())
            .unwrap_or(u64::MAX)
            .saturating_sub(rel);
        Some(available.min(range.len))
    }

    fn serve_need(&mut self, range: ByteRange) -> Result<bool> {
        match self.source {
            SourceMode::Seekable => {
                let Some((offset, bytes)) = self.at.take() else {
                    return Ok(false);
                };
                if offset != range.offset {
                    return Err(Error::InvalidInput(
                        "matroska: supplied offset does not match the reader's need",
                    ));
                }
                let len = usize::try_from(range.len).unwrap_or(usize::MAX);
                let slice = if bytes.len() > len {
                    &bytes[..len]
                } else {
                    &bytes[..]
                };
                let short = u64::try_from(slice.len()).unwrap_or(u64::MAX) < range.len;
                self.reader.supply(range.offset, slice)?;
                if short {
                    self.eof = true;
                }
                Ok(true)
            }
            SourceMode::Sequential => {
                let eof = self.eof;
                let available = self.covered(range);
                let answer = match available {
                    Some(available) if available == range.len || eof => available,
                    _ => return Ok(false),
                };
                let rel = usize::try_from(range.offset - self.buf_start).unwrap_or(usize::MAX);
                let take = usize::try_from(answer).unwrap_or(usize::MAX);
                let slice = self.buf.get(rel..rel.saturating_add(take)).unwrap_or(&[]);
                self.reader.supply(range.offset, slice)?;
                self.prune(range.offset);
                Ok(true)
            }
        }
    }

    fn take_frame(&mut self, block: BlockStep) -> Result<bool> {
        match self.source {
            SourceMode::Seekable => {
                let Some((offset, bytes)) = self.at.take() else {
                    return Ok(false);
                };
                if offset != block.frame.offset {
                    return Err(Error::InvalidInput(
                        "matroska: supplied offset does not match the frame's range",
                    ));
                }
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX) < block.frame.len {
                    self.push_truncated(block.frame.offset);
                    self.done = true;
                    return Ok(true);
                }
                let len = usize::try_from(block.frame.len).unwrap_or(usize::MAX);
                let data = if bytes.len() > len {
                    bytes.slice(..len)
                } else {
                    bytes
                };
                self.on_frame(block, data)?;
                Ok(true)
            }
            SourceMode::Sequential => {
                let eof = self.eof;
                let available = self.covered(block.frame);
                let Some(available) = available else {
                    return Err(Error::InvalidInput(
                        "matroska: a frame lies behind the sequential buffer",
                    ));
                };
                if available < block.frame.len && !eof {
                    return Ok(false);
                }
                if available < block.frame.len {
                    self.push_truncated(block.frame.offset);
                    self.done = true;
                    return Ok(true);
                }
                let rel =
                    usize::try_from(block.frame.offset - self.buf_start).unwrap_or(usize::MAX);
                let take = usize::try_from(block.frame.len).unwrap_or(usize::MAX);
                let slice = self
                    .buf
                    .get(rel..rel.saturating_add(take))
                    .ok_or(Error::InvalidInput("matroska: frame range is not buffered"))?;
                let data = Bytes::copy_from_slice(slice);
                self.prune(block.frame.offset);
                self.on_frame(block, data)?;
                Ok(true)
            }
        }
    }

    fn prune(&mut self, served_offset: u64) {
        let floor = served_offset.saturating_sub(self.retain);
        if floor <= self.buf_start {
            return;
        }
        let drop = usize::try_from(floor - self.buf_start).unwrap_or(usize::MAX);
        let drop = drop.min(self.buf.len());
        let _ = self.buf.split_to(drop);
        self.buf_start = self.buf_start.saturating_add(drop as u64);
    }

    fn push_warning(&mut self, warning: Warning) {
        let position = warning.position();
        let kind = match warning {
            Warning::Truncated { .. } => InputDegradation::TruncatedUnit { position },
            _ => InputDegradation::MalformedUnit { position },
        };
        self.events.push_back(DemuxEvent::InputDegraded {
            track: None,
            kind,
            provenance: EventProvenance::default(),
        });
    }

    fn push_truncated(&mut self, position: u64) {
        self.events.push_back(DemuxEvent::InputDegraded {
            track: None,
            kind: InputDegradation::TruncatedUnit { position },
            provenance: EventProvenance::default(),
        });
    }
}

impl Stage for MatroskaDemux {
    type In<'a> = MatroskaInput<'a>;
    type Out = DemuxEvent;
    type Error = Error;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<()> {
        if let Some(err) = &self.poisoned {
            return Err(err.clone());
        }
        match input {
            MatroskaInput::Eof => {
                if self.finished {
                    return Ok(());
                }
                self.eof = true;
                self.pump()
            }
            MatroskaInput::At { offset, bytes } => {
                if self.finished {
                    return Err(Error::InvalidInput("MatroskaDemux was fed after finish"));
                }
                if self.eof {
                    return Err(Error::InvalidInput("MatroskaDemux was fed after Eof"));
                }
                if self.source != SourceMode::Seekable {
                    return Err(Error::InvalidInput(
                        "MatroskaInput::At was fed to a sequential MatroskaDemux",
                    ));
                }
                self.at = Some((offset, bytes));
                self.pump()
            }
            MatroskaInput::Next(data) => {
                if self.finished {
                    return Err(Error::InvalidInput("MatroskaDemux was fed after finish"));
                }
                if self.eof {
                    return Err(Error::InvalidInput("MatroskaDemux was fed after Eof"));
                }
                if self.source != SourceMode::Sequential {
                    return Err(Error::InvalidInput(
                        "MatroskaInput::Next was fed to a seekable MatroskaDemux",
                    ));
                }
                self.buf.extend_from_slice(data);
                self.pump()
            }
        }
    }

    fn poll(&mut self) -> Option<DemuxEvent> {
        self.events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(err) = &self.poisoned {
            return Err(err.clone());
        }
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.eof = true;
        if let Err(err) = self.pump() {
            if !matches!(
                err,
                Error::Matroska(broadcast_matroska::Error::Truncated { .. })
            ) {
                return Err(err);
            }
            let position = self
                .frame
                .map(|block| block.frame.offset)
                .or(self.need.map(|range| range.offset))
                .unwrap_or(self.buf_start);
            self.frame = None;
            self.need = None;
            self.push_truncated(position);
            self.done = true;
        }
        self.finish_flush()
    }

    fn demand(&self) -> Demand {
        if self.poisoned.is_some() || self.finished || self.done {
            return Demand::saturated();
        }
        match self.source {
            SourceMode::Seekable => {
                if let Some(block) = self.frame {
                    if block.frame.len == 0 {
                        return Demand::saturated();
                    }
                    return Demand::at(
                        block.frame.offset,
                        usize::try_from(block.frame.len).unwrap_or(usize::MAX),
                    );
                }
                if let Some(range) = self.need {
                    if range.len == 0 {
                        return Demand::saturated();
                    }
                    return Demand::at(
                        range.offset,
                        usize::try_from(range.len).unwrap_or(usize::MAX),
                    );
                }
                Demand::saturated()
            }
            SourceMode::Sequential => Demand::new(0),
        }
    }
}

/// A Block's presentation time in IR ticks (milliseconds), truncating toward
/// zero exactly as the whole-input demuxer did.
fn block_pts(block: &BlockStep) -> Result<i64> {
    i64::try_from(block.timestamp_ns / IR_TICK_NS)
        .map_err(|_| Error::InvalidInput("matroska: block timestamp does not fit the IR"))
}

/// The first positive Block gap divided by the earlier Block's frame count
/// (RFC 9559 §11): the only cadence a TrackEntry without `DefaultDuration`
/// leaves. A same-timestamp pair contributes no interval and is skipped.
fn derive_nominal(blocks: &VecDeque<HeldBlock>) -> Option<u64> {
    let mut previous = blocks.front();
    for current in blocks.iter().skip(1) {
        if let Some(earlier) = previous {
            let frames = earlier.frames.len();
            let gap = current.pts.saturating_sub(earlier.pts);
            if frames > 0 && gap > 0 {
                return Some(u64::try_from(gap).unwrap_or(0) / u64::try_from(frames).unwrap_or(1));
            }
        }
        previous = Some(current);
    }
    None
}

/// Convert a `DefaultDuration` in nanoseconds to IR ticks, rounding to nearest.
pub fn ns_to_ir_ticks(ns: u64) -> u64 {
    rescale_u64(ns, NS_TIMESCALE, IR_TIMESCALE, Rounding::Nearest)
}

/// The opaque Matroska identity of a CodecID this crate does not map.
fn data_config(info: &TrackInfo) -> CodecConfig {
    CodecConfig::Data {
        identity: DataIdentity::Matroska {
            codec_id: String::from(info.codec_id()),
            codec_private: Bytes::copy_from_slice(info.codec_private()),
        },
    }
}

/// Map a [`TrackInfo`] to a [`CodecConfig`] plus, for the length-prefixed
/// codecs (AVC/HEVC), the NAL length-prefix size its `CodecPrivate` declares —
/// or `None` for a CodecID this crate does not map.
fn codec_config_for(
    info: &TrackInfo,
    first_frame: &[u8],
) -> Result<Option<(CodecConfig, Option<usize>)>> {
    let codec = info.codec_id();
    let video = info.track_type_value() == TRACK_TYPE_VIDEO;
    let audio = info.track_type_value() == TRACK_TYPE_AUDIO;
    if video && codec == CODEC_V_VP9 {
        Ok(Some((vp9_config(info), None)))
    } else if video && codec == CODEC_V_VP8 {
        Ok(Some((vp8_config(first_frame)?, None)))
    } else if video && codec == CODEC_V_AVC {
        Ok(Some(avc_config(info)?))
    } else if video && codec == CODEC_V_HEVC {
        Ok(Some(hevc_config(info)?))
    } else if audio && codec == CODEC_A_OPUS {
        Ok(Some((opus_config(info)?, None)))
    } else if audio && codec == CODEC_A_VORBIS {
        Ok(Some((vorbis_config(info)?, None)))
    } else if audio && codec == CODEC_A_AAC {
        Ok(Some((
            aac_config_from_asc_bytes(info.codec_private().to_vec())?,
            None,
        )))
    } else {
        Ok(None)
    }
}

/// A `Video` element dimension as the IR's `u16`.
fn dimension(value: Option<u64>) -> u16 {
    u16::try_from(value.unwrap_or(0)).unwrap_or(u16::MAX)
}

fn avc_config(info: &TrackInfo) -> Result<(CodecConfig, Option<usize>)> {
    let mut record = AVCDecoderConfigurationRecord::parse(info.codec_private())?;
    let length_size = usize::from(record.length_size_minus_one) + 1;
    record.length_size_minus_one = NAL_LENGTH_SIZE_MINUS_ONE;
    Ok((
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: dimension(info.pixel_width()),
            height: dimension(info.pixel_height()),
        },
        Some(length_size),
    ))
}

fn hevc_config(info: &TrackInfo) -> Result<(CodecConfig, Option<usize>)> {
    let mut record = HEVCDecoderConfigurationRecord::parse(info.codec_private())?;
    let length_size = usize::from(record.length_size_minus_one) + 1;
    record.length_size_minus_one = NAL_LENGTH_SIZE_MINUS_ONE;
    Ok((
        CodecConfig::Hevc {
            config: HEVCConfigurationBox::new(record),
            width: dimension(info.pixel_width()),
            height: dimension(info.pixel_height()),
        },
        Some(length_size),
    ))
}

/// Build a [`CodecConfig::Vp8`] from the VP8 key-frame header (RFC 6386 §9.1):
/// a 3-byte frame tag whose `key_frame` bit is `0`, the start code
/// `0x9D 01 2A`, then two 16-bit little-endian dimension words.
fn vp8_config(first_frame: &[u8]) -> Result<CodecConfig> {
    need_in(first_frame, VP8_KEYFRAME_HEADER_LEN, "VP8 key-frame header")?;
    if first_frame[0] & VP8_KEYFRAME_TAG_BIT != 0 {
        return Err(Error::InvalidValue(InvalidValue {
            field: "VP8 key_frame",
            value: u64::from(first_frame[0] & VP8_KEYFRAME_TAG_BIT),
            reason: "first VP8 frame is not a key frame (key_frame bit != 0)",
        }));
    }
    let start = &first_frame[VP8_FRAME_TAG_LEN..VP8_FRAME_TAG_LEN + VP8_START_CODE.len()];
    if start != VP8_START_CODE {
        return Err(Error::InvalidValue(InvalidValue {
            field: "VP8 start code",
            value: start
                .iter()
                .fold(0u32, |acc, &b| (acc << u8::BITS) | u32::from(b)) as u64,
            reason: "VP8 key-frame start code is not 0x9D012A",
        }));
    }
    let d = VP8_FRAME_TAG_LEN + VP8_START_CODE.len();
    let width = u16::from_le_bytes([first_frame[d], first_frame[d + 1]]) & VP8_DIMENSION_MASK;
    let height_at = d + VP8_DIMENSION_LEN;
    let height = u16::from_le_bytes([first_frame[height_at], first_frame[height_at + 1]])
        & VP8_DIMENSION_MASK;
    Ok(CodecConfig::Vp8 { width, height })
}

/// Build a [`CodecConfig::Vorbis`] from a Vorbis [`TrackInfo`]: `CodecPrivate`
/// is stored verbatim and the Identification header (Vorbis I §4.2.2) gives
/// channels and sample rate.
fn vorbis_config(info: &TrackInfo) -> Result<CodecConfig> {
    let cp = info.codec_private();
    let id = vorbis_id_header(cp)?;
    need_in(id, VORBIS_ID_MIN_LEN, "Vorbis identification header")?;
    if id[0] != VORBIS_ID_HEADER_TYPE {
        return Err(Error::InvalidValue(InvalidValue {
            field: "Vorbis header packet type",
            value: u64::from(id[0]),
            reason: "first Vorbis header is not the identification header (type 0x01)",
        }));
    }
    if &id[1..1 + VORBIS_SIGNATURE.len()] != VORBIS_SIGNATURE {
        return Err(Error::InvalidInput(
            "Vorbis identification header missing the \"vorbis\" signature",
        ));
    }
    let channels = u16::from(id[VORBIS_ID_CHANNELS_OFFSET]);
    let sample_rate = u32::from_le_bytes(
        id[VORBIS_ID_SAMPLE_RATE_OFFSET..VORBIS_ID_MIN_LEN]
            .try_into()
            .unwrap_or_default(),
    );
    Ok(CodecConfig::Vorbis {
        codec_private: cp.to_vec(),
        channels,
        sample_rate,
    })
}

/// Slice the Vorbis Identification header out of the Xiph-laced `CodecPrivate`
/// (Vorbis I §4.2.2): byte 0 is `numPackets - 1`, then the first two headers'
/// lengths as continuation runs, then the packets themselves.
fn vorbis_id_header(cp: &[u8]) -> Result<&[u8]> {
    need_in(cp, 1, "Vorbis CodecPrivate (Xiph lacing count)")?;
    if cp[0] != VORBIS_LACE_COUNT {
        return Err(Error::InvalidValue(InvalidValue {
            field: "Vorbis CodecPrivate lacing count",
            value: u64::from(cp[0]),
            reason: "expected numPackets-1 == 2 (three Xiph-laced Vorbis headers)",
        }));
    }
    let mut pos = 1usize;
    let mut lengths = [0usize; VORBIS_LACING_LEN_FIELDS];
    for len in lengths.iter_mut() {
        loop {
            let byte = *cp.get(pos).ok_or(Error::BufferTooShort(BufferTooShort {
                need: pos + 1,
                have: cp.len(),
                what: "Vorbis CodecPrivate Xiph lacing length",
            }))?;
            pos += 1;
            *len += usize::from(byte);
            if byte != XIPH_LACE_CONTINUATION {
                break;
            }
        }
    }
    let id_start = pos;
    let id_end = id_start
        .checked_add(lengths[0])
        .filter(|&end| end <= cp.len())
        .ok_or(Error::BufferTooShort(BufferTooShort {
            need: id_start + lengths[0],
            have: cp.len(),
            what: "Vorbis identification header body",
        }))?;
    Ok(&cp[id_start..id_end])
}

/// Build a [`CodecConfig::Vp9`] with WebM's documented profile-0 / 8-bit
/// default `vpcC` (WebM stores no `vpcC` in `CodecPrivate` for VP9).
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
        width: dimension(info.pixel_width()),
        height: dimension(info.pixel_height()),
    }
}

/// Build a [`CodecConfig::Opus`] by parsing the `OpusHead` identification
/// header carried in `CodecPrivate` (RFC 7845 §5.1).
fn opus_config(info: &TrackInfo) -> Result<CodecConfig> {
    let cp = info.codec_private();
    need_in(cp, OPUS_HEAD_MIN_LEN, "Opus CodecPrivate (OpusHead)")?;
    if &cp[0..OPUS_HEAD_MAGIC.len()] != OPUS_HEAD_MAGIC {
        return Err(Error::InvalidValue(InvalidValue {
            field: "OpusHead magic",
            value: be_u64(cp, 0, "Opus CodecPrivate (OpusHead)")?,
            reason: "Opus CodecPrivate does not start with the \"OpusHead\" signature",
        }));
    }
    let version = cp[OPUS_HEAD_VERSION_OFFSET];
    let output_channel_count = cp[OPUS_HEAD_CHANNELS_OFFSET];
    let pre_skip = u16::from_le_bytes([
        cp[OPUS_HEAD_PRE_SKIP_OFFSET],
        cp[OPUS_HEAD_PRE_SKIP_OFFSET + 1],
    ]);
    let input_sample_rate = u32::from_le_bytes(
        cp[OPUS_HEAD_INPUT_RATE_OFFSET..OPUS_HEAD_OUTPUT_GAIN_OFFSET]
            .try_into()
            .unwrap_or_default(),
    );
    let output_gain = i16::from_le_bytes([
        cp[OPUS_HEAD_OUTPUT_GAIN_OFFSET],
        cp[OPUS_HEAD_OUTPUT_GAIN_OFFSET + 1],
    ]);
    let channel_mapping_family = cp[OPUS_HEAD_MAPPING_FAMILY_OFFSET];
    let channel_mapping = if channel_mapping_family != 0 {
        let need = OPUS_HEAD_CHANNEL_MAPPING_OFFSET + usize::from(output_channel_count);
        need_in(cp, need, "OpusHead channel-mapping table")?;
        let stream_count = cp[OPUS_HEAD_STREAM_COUNT_OFFSET];
        let coupled_count = cp[OPUS_HEAD_COUPLED_COUNT_OFFSET];
        let map_end = OPUS_HEAD_CHANNEL_MAPPING_OFFSET + usize::from(output_channel_count);
        Some(ChannelMappingTable {
            stream_count,
            coupled_count,
            channel_mapping: cp[OPUS_HEAD_CHANNEL_MAPPING_OFFSET..map_end].to_vec(),
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
        channel_count: u16::from(output_channel_count),
        sample_rate: OPUS_OUTPUT_SAMPLE_RATE,
        sample_size: AUDIO_SAMPLE_SIZE,
    })
}
