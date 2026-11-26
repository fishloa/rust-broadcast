//! MPEG-2 Program Stream streaming demuxer - the PS input side of the
//! any-to-any container hub.
//!
//! [`StreamingPsDemux`] is a [`Stage`] over [`mpeg_ps::PsScanner`]: it is fed
//! the Program Stream in pieces and emits the hub's [`DemuxEvent`] vocabulary
//! (track additions and one [`Sample`] per completed access unit) as each PES
//! packet / access unit completes. It never holds the input: the scanner holds
//! at most one incomplete PES, and each elementary stream holds one access unit
//! of lookahead (the sample's duration is the gap to the next decode time).
//!
//! Pipeline: PS pack layer ([`mpeg_ps`]) -> per-`stream_id` PES reassembly
//! ([`mpeg_pes`]) -> codec-config recovery (H.264 in-band SPS/PPS -> `avcC`,
//! MPEG-2 `sequence_header()`, AC-3 syncframe BSI -> `dac3`) -> length-prefixed
//! video / raw audio samples.
//!
//! Unlike a Transport Stream, a Program Stream carries no PMT here: the fixture
//! has no [`ProgramStreamMap`](mpeg_ps::ProgramStreamMap), so elementary streams
//! are mapped by `stream_id` (ISO/IEC 13818-1 Table 2-22): video from the video
//! range 0xE0-0xEF, audio from `private_stream_1` (0xBD). A PSM, when present,
//! restricts the set to the elementary streams it lists. Unknown `stream_id`s
//! are skipped, never fatal.
//!
//! A `private_stream_1` `stream_id` is a *container*: it multiplexes several
//! independent substreams (AC-3, DTS, LPCM, subpictures), told apart by the
//! `substream_id` byte at the front of each PES payload's 4-byte substream
//! header. Elementary streams are therefore keyed by `(stream_id,
//! substream_id)`. A substream is carried only when its own bytes identify it
//! as AC-3: the syncword is looked for at the header's
//! `first_access_unit_pointer`, and the DTS (`0x88..=0x8F`), LPCM
//! (`0xA0..=0xA7`) and subpicture (`0x20..=0x3F`) substream ids are skipped
//! explicitly rather than concatenated into the AC-3 track beside them.
//!
//! A PS also does not stamp every frame: one PES packet concatenates several
//! access units and carries a PTS/DTS only for the first one. Video access units
//! are therefore recovered incrementally by
//! [`crate::au::AccessUnitSplitter`] (a boundary is a new picture, from
//! `first_mb_in_slice` as well as from an access-unit delimiter - an AUD is
//! *optional* in H.264 and absent from most PS captures), and MPEG-2 pictures by
//! their `picture_start_code` (`0x00000100`, ISO/IEC 13818-2 §6.2.3). Audio is
//! split into AC-3 syncframes by walking each frame's declared length rather
//! than scanning for the next syncword. Timing is anchored off the PES-level
//! PTS/DTS that are present and the constant frame duration derived from the
//! stamped decode timestamps.
//!
//! HEVC / DTS / other codecs are not carried here (skipped, never fatal).
//!
//! # Spec
//!
//! - **Program Stream framing (pack header / system header / PSM)**: ISO/IEC
//!   13818-1 (ITU-T H.222.0) §2.5 - via [`mpeg_ps`].
//! - **PES reassembly + PTS/DTS**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7 (via
//!   [`mpeg_pes`], 33-bit @ 90 kHz).
//! - **`stream_id` assignment**: ISO/IEC 13818-1 Table 2-22.
//! - **AC-3 in `private_stream_1`**: ETSI TS 101 154 - the 4-byte substream
//!   header (`substream_id` + `number_of_frames` + `first_access_unit_pointer`)
//!   precedes the AC-3 syncframes.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::num::NonZeroU32;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use broadcast_core::timescale::Timescale;
use broadcast_core::wire::u16_at;
use mpeg_ps::{PsEvent, PsLimits, PsScanner};

use crate::TrackId;
use crate::ac3_syncframe::{AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo};
use crate::au::AccessUnitSplitter;
use crate::display_track::{SpsTracker, TrackCodec};
use crate::error::{Error, Result};
use crate::ir::{
    AUDIO_SAMPLE_SIZE_BITS, DemuxEvent, DisplayGeometry, EventProvenance, InputDegradation, Sample,
    SampleFlags, TrackSpec,
};
use crate::mpeg_legacy::Mpeg2SeqHeader;
use crate::mpeg2_display::Mpeg2DisplayTracker;
use crate::nal::{
    AVC_NAL_IDR, AVC_NAL_PPS, AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, ParamKind,
    parameter_set_is_valid,
};
use crate::pipeline::CodecConfig;
use crate::ts_demux::{
    AVC_SPS_CONFIG_MIN_LEN, AVC_SPS_CONSTRAINT_FLAGS_OFFSET, AVC_SPS_LEVEL_IDC_OFFSET,
    MPEG2_PICTURE_START_CODE, mpeg2_is_sync,
};
use broadcast_bitstream::annexb::iter_annexb_nals;
use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use isobmff::nalu_types::{AvcPps, AvcSps};

// -- stream_id -> codec (ISO/IEC 13818-1 Table 2-22) --------------------------

/// Low bound of the H.264/video `stream_id` range (`1110 xxxx`, 0xE0-0xEF).
const STREAM_ID_VIDEO_LO: u8 = 0xE0;
/// High bound of the video `stream_id` range.
const STREAM_ID_VIDEO_HI: u8 = 0xEF;
/// `private_stream_1` `stream_id` - carries user-private payloads, in practice
/// AC-3/E-AC-3/DTS/LPCM audio and subpictures (Table 2-22; H.222.0 §2.4.3.7
/// makes its `PES_packet_data_byte`s "user definable").
const STREAM_ID_PRIVATE_1: u8 = 0xBD;

/// Length of the `private_stream_1` substream header before the substream's own
/// payload: `substream_id`(8) + `number_of_frames`(8) +
/// `first_access_unit_pointer`(16) - the `private_stream_1`-specific
/// `PES_packet_data_byte` layout (H.222.0 §2.4.3.7 leaves its *contents* to the
/// application, but this 4-byte prologue is what the DVD/ATSC private-stream
/// profiles this demuxer targets put there).
const PRIVATE1_HEADER_LEN: usize = 4;
/// Offset of the `substream_id` byte within that header.
const PRIVATE1_SUBSTREAM_ID_OFFSET: usize = 0;
/// Offset of `number_of_frames` within the substream header.
const PRIVATE1_NUM_FRAMES_OFFSET: usize = 1;
/// Offset of `first_access_unit_pointer` within the substream header.
const PRIVATE1_FIRST_ACCESS_UNIT_POINTER_OFFSET: usize = 2;

/// DVD/ATSC subpicture substream ids (`0x20..=0x3F`), never carried.
const SUBSTREAM_ID_SUBPICTURE_RANGE: core::ops::RangeInclusive<u8> = 0x20..=0x3F;
/// DVD/ATSC DTS substream ids (`0x88..=0x8F`), never carried.
const SUBSTREAM_ID_DTS_RANGE: core::ops::RangeInclusive<u8> = 0x88..=0x8F;
/// DVD/ATSC LPCM substream ids (`0xA0..=0xA7`), never carried.
const SUBSTREAM_ID_LPCM_RANGE: core::ops::RangeInclusive<u8> = 0xA0..=0xA7;
/// Audio substream ids start here (`0x80`); below it nothing is AC-3.
const SUBSTREAM_ID_AUDIO_LO: u8 = 0x80;
/// The `substream_id` value used for a `stream_id` that is not a substream
/// container.
const PRIVATE1_SUBSTREAM_ID_NONE: u8 = 0;

/// The AC-3 syncword (`0x0B77`, ETSI TS 102 366 §4.4.1.1).
const AC3_SYNCWORD: [u8; 2] = [0x0B, 0x77];

/// The three-byte start-code prefix.
const START_CODE_PREFIX: [u8; 3] = [0x00, 0x00, 0x01];

/// Byte width of a start code: the `00 00 01` prefix plus its type byte.
const START_CODE_LEN: usize = core::mem::size_of::<u32>();

/// Byte offset of a start code's type byte within `00 00 01 xx`.
const START_CODE_BYTE_OFFSET: usize = core::mem::size_of::<u32>() - 1;

/// Samples a decode-time gap can be derived from.
const SAMPLES_PER_DURATION: usize = 2;

/// The video clock: 90 kHz, the PES/PS timestamp timescale
/// (ISO/IEC 13818-1 §2.4.3.7).
const VIDEO_TIMESCALE: Timescale = broadcast_core::timescale!(crate::rtp::VIDEO_CLOCK_RATE);

/// The frame duration assumed when fewer than two decode timestamps are known.
const DEFAULT_FRAME_DURATION: i128 = 3600;

/// The caller's hostile-input bounds for [`StreamingPsDemux`].
///
/// Every bound is a cap on what one stream may hold while it waits for a
/// configuration unit. They are not policy: a conformant stream resolves its
/// config in its first access unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PsDemuxLimits {
    /// Largest elementary-stream prefix buffered while a video codec / config is
    /// still undecided. Default 1 MiB.
    pub max_probe_bytes: usize,
    /// Most samples one elementary stream may hold while an earlier stream has
    /// not yet announced (track order is first-seen order). Default 4096.
    pub max_held_samples: usize,
    /// The PES scanner's own bounds.
    pub scanner: PsLimits,
}

impl PsDemuxLimits {
    /// The hostile-input defaults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_probe_bytes: 1 << 20,
            max_held_samples: 4096,
            scanner: PsLimits::new(),
        }
    }
}

impl Default for PsDemuxLimits {
    fn default() -> Self {
        Self::new()
    }
}

/// Codec class recovered from a `stream_id`. Data-carrying dispatch discriminant,
/// not a spec label enum - hence no `name()`/`Display` (see the
/// `workspace-guards/tests/guards/label.rs` policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    /// A `stream_id` in the video range (Table 2-22, 0xE0-0xEF). This range
    /// carries H.264 *or* MPEG-2 video (or other video codecs this demuxer
    /// doesn't support) indistinguishably by `stream_id` alone - the actual
    /// codec is decided by probing the elementary bytes (C6, #1009).
    Video,
    /// A `private_stream_1` (0xBD) PES: a container for several substreams,
    /// each identified by the first byte of its substream header. Resolved
    /// per substream by [`Codec::classify_private1`] into [`Codec::Ac3`] or
    /// [`Codec::Skipped`].
    Private,
    Ac3,
    /// A substream this demuxer does not carry. Skipped, never fatal - it
    /// produces no track and accumulates no bytes.
    Skipped,
}

impl Codec {
    /// Map a PES `stream_id` to a supported [`Codec`] *without* inspecting the
    /// payload, or `None` for a `stream_id` this demuxer never carries.
    fn from_stream_id(stream_id: u8) -> Option<Self> {
        match stream_id {
            STREAM_ID_VIDEO_LO..=STREAM_ID_VIDEO_HI => Some(Codec::Video),
            STREAM_ID_PRIVATE_1 => Some(Codec::Private),
            _ => None,
        }
    }

    /// Classify one `private_stream_1` substream from its header and the payload
    /// that follows it, or `None` to skip it.
    fn classify_private1(
        substream_id: u8,
        header: &Private1Header,
        payload: &[u8],
    ) -> Option<Self> {
        if SUBSTREAM_ID_SUBPICTURE_RANGE.contains(&substream_id)
            || SUBSTREAM_ID_DTS_RANGE.contains(&substream_id)
            || SUBSTREAM_ID_LPCM_RANGE.contains(&substream_id)
        {
            return None;
        }
        if substream_id < SUBSTREAM_ID_AUDIO_LO {
            return None;
        }
        let pointer = usize::from(header.first_access_unit_pointer);
        let offset = pointer.saturating_sub(1);
        let probe = payload.get(offset..)?;
        if probe.len() < AC3_SYNCWORD.len() || !probe.starts_with(&AC3_SYNCWORD) {
            return None;
        }
        Some(Codec::Ac3)
    }
}

/// The 4-byte `private_stream_1` substream header that precedes each substream's
/// own bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Private1Header {
    /// `substream_id` - distinguishes the substreams sharing one 0xBD
    /// `stream_id`.
    substream_id: u8,
    /// `number_of_frames` - how many complete access units start in this PES
    /// packet (used only for header consistency).
    number_of_frames: u8,
    /// `first_access_unit_pointer` - byte offset, from the byte after this
    /// field, of the first access unit that begins in this packet; 0 when none
    /// does.
    first_access_unit_pointer: u16,
}

impl Private1Header {
    /// Parse the 4-byte substream header at the front of a `private_stream_1`
    /// PES payload, returning it and the substream's own bytes.
    fn parse(payload: &[u8]) -> Option<(Self, &[u8])> {
        if payload.len() < PRIVATE1_HEADER_LEN {
            return None;
        }
        let header = Private1Header {
            substream_id: payload[PRIVATE1_SUBSTREAM_ID_OFFSET],
            number_of_frames: payload[PRIVATE1_NUM_FRAMES_OFFSET],
            first_access_unit_pointer: u16_at(payload, PRIVATE1_FIRST_ACCESS_UNIT_POINTER_OFFSET),
        };
        Some((header, &payload[PRIVATE1_HEADER_LEN..]))
    }

    /// True if the payload can hold the access units this header claims.
    fn is_consistent(&self, payload_len: usize) -> bool {
        let body_len = payload_len.saturating_sub(PRIVATE1_HEADER_LEN);
        let pointer = usize::from(self.first_access_unit_pointer);
        if self.number_of_frames > 0 && pointer == 0 {
            return false;
        }
        if pointer > body_len.saturating_add(1) {
            return false;
        }
        true
    }
}

/// Identifies one elementary stream inside the program stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StreamKey {
    stream_id: u8,
    /// The `private_stream_1` `substream_id`; meaningless (and
    /// [`PRIVATE1_SUBSTREAM_ID_NONE`]) for a `stream_id` that is not a
    /// substream container.
    substream_id: u8,
}

/// A PES-level timestamp anchored at a byte offset within a reassembled
/// elementary stream, already wrap-unrolled (90 kHz).
#[derive(Debug, Clone, Copy)]
struct Stamp {
    offset: u64,
    pts: Option<i128>,
    dts: Option<i128>,
}

/// A streaming MPEG-1/2 Program Stream demuxer.
///
/// Feed it bytes with [`Stage::feed`]; drain [`DemuxEvent`]s with
/// [`Stage::poll`]; call [`Stage::finish`] at end of input so the trailing
/// access unit and any pending samples are released.
pub struct StreamingPsDemux {
    scanner: PsScanner,
    state: DemuxState,
    finished: bool,
    /// Total bytes fed, for absolute degradation positions.
    fed: u64,
}

impl Default for StreamingPsDemux {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingPsDemux {
    /// A demuxer with the default [`PsDemuxLimits`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(PsDemuxLimits::default())
    }

    /// A demuxer bounded by `limits`.
    #[must_use]
    pub fn with_limits(limits: PsDemuxLimits) -> Self {
        Self {
            scanner: PsScanner::new(limits.scanner),
            state: DemuxState::new(limits),
            finished: false,
            fed: 0,
        }
    }

    /// Drain every unit the scanner has ready.
    fn drain(&mut self) -> Result<()> {
        loop {
            let buffered = self.scanner.buffered_len();
            let position = self.fed.saturating_sub(buffered as u64);
            let Some(event) = self.scanner.poll()? else {
                break;
            };
            match event {
                PsEvent::Pack(header) => {
                    self.state.packs_seen += 1;
                    let _ = header;
                }
                PsEvent::SystemHeader(_) => {}
                PsEvent::ProgramStreamMap(map) => self.state.on_psm(&map),
                PsEvent::Pes { stream_id, packet } => {
                    self.state.on_pes(stream_id, packet, position)?;
                }
                PsEvent::Resync { skipped } => {
                    self.state.on_resync(position, skipped);
                }
                PsEvent::End => {}
                _ => {}
            }
        }
        Ok(())
    }
}

impl Stage for StreamingPsDemux {
    type In<'a> = &'a [u8];
    type Out = DemuxEvent;
    type Error = Error;

    fn feed(&mut self, input: &'_ [u8], _now: Timestamp) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.fed += input.len() as u64;
        self.scanner.feed(input);
        self.drain()
    }

    fn poll(&mut self) -> Option<DemuxEvent> {
        self.state.events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.scanner.finish();
        self.drain()?;
        if self.state.packs_seen == 0
            && let Some(error) = self.scanner.first_error()
        {
            return Err(Error::Ps(error.clone()));
        }
        self.state.finish_streams()?;
        self.finished = true;
        Ok(())
    }
}

/// Everything the demuxer owns except the byte scanner.
struct DemuxState {
    events: VecDeque<DemuxEvent>,
    streams: Vec<Stream>,
    by_key: BTreeMap<StreamKey, usize>,
    next_track: TrackId,
    /// Streams below this index have announced (or can never announce): the
    /// track order is first-seen order, as the whole-buffer demuxer's was.
    announce_cursor: usize,
    limits: PsDemuxLimits,
    packs_seen: u64,
    /// When a Program Stream Map is present, the elementary streams it lists.
    psm_ids: Option<BTreeSet<u8>>,
}

impl DemuxState {
    fn new(limits: PsDemuxLimits) -> Self {
        Self {
            events: VecDeque::new(),
            streams: Vec::new(),
            by_key: BTreeMap::new(),
            next_track: TrackId::new(1),
            announce_cursor: 0,
            limits,
            packs_seen: 0,
            psm_ids: None,
        }
    }

    fn on_psm(&mut self, map: &mpeg_ps::ProgramStreamMap<'_>) {
        self.psm_ids = Some(
            map.elementary_stream_map
                .iter()
                .map(|entry| entry.elementary_stream_id)
                .collect(),
        );
    }

    fn on_resync(&mut self, position: u64, skipped: u64) {
        let _ = skipped;
        self.events.push_back(DemuxEvent::input_degraded(
            None,
            InputDegradation::MalformedUnit { position },
            EventProvenance::default(),
        ));
    }

    fn stream_index(&mut self, key: StreamKey) -> usize {
        if let Some(&index) = self.by_key.get(&key) {
            return index;
        }
        let index = self.streams.len();
        let (codec, body) = if key.stream_id == STREAM_ID_PRIVATE_1 {
            (Codec::Private, Body::Ac3(Ac3State::new()))
        } else {
            (Codec::Video, Body::VideoProbe(Vec::new()))
        };
        self.streams.push(Stream::new(codec, body));
        self.by_key.insert(key, index);
        index
    }

    fn on_pes(&mut self, stream_id: u8, packet: &[u8], position: u64) -> Result<()> {
        let Some(codec) = Codec::from_stream_id(stream_id) else {
            return Ok(());
        };
        if let Some(ids) = &self.psm_ids
            && !ids.contains(&stream_id)
        {
            return Ok(());
        }
        let pes = match mpeg_pes::PesPacket::parse(packet) {
            Ok(pes) => pes,
            Err(_) => {
                self.events.push_back(DemuxEvent::input_degraded(
                    None,
                    InputDegradation::MalformedUnit { position },
                    EventProvenance::default(),
                ));
                return Ok(());
            }
        };
        let (pts, dts) = pes
            .header
            .as_ref()
            .map(|header| {
                (
                    header.pts.map(|pts| pts.ticks()),
                    header.dts.map(|dts| dts.ticks()),
                )
            })
            .unwrap_or((None, None));
        match codec {
            Codec::Video => {
                if pes.payload.is_empty() {
                    return Ok(());
                }
                let key = StreamKey {
                    stream_id,
                    substream_id: PRIVATE1_SUBSTREAM_ID_NONE,
                };
                let index = self.stream_index(key);
                self.streams[index].push_video(pes.payload, pts, dts, &self.limits)?;
                self.try_announce();
                Ok(())
            }
            Codec::Private => self.on_private_pes(stream_id, &pes, pts, dts),
            Codec::Ac3 | Codec::Skipped => Ok(()),
        }
    }

    fn on_private_pes(
        &mut self,
        stream_id: u8,
        pes: &mpeg_pes::PesPacket<'_>,
        pts: Option<u64>,
        dts: Option<u64>,
    ) -> Result<()> {
        let Some((header, body)) = Private1Header::parse(pes.payload) else {
            return Ok(());
        };
        let key = StreamKey {
            stream_id,
            substream_id: header.substream_id,
        };
        let index = self.stream_index(key);
        let stream = &mut self.streams[index];
        if stream.codec == Codec::Private {
            let named_skip = SUBSTREAM_ID_SUBPICTURE_RANGE.contains(&header.substream_id)
                || SUBSTREAM_ID_DTS_RANGE.contains(&header.substream_id)
                || SUBSTREAM_ID_LPCM_RANGE.contains(&header.substream_id)
                || header.substream_id < SUBSTREAM_ID_AUDIO_LO;
            if named_skip {
                stream.codec = Codec::Skipped;
                stream.resolved = true;
                self.try_announce();
                return Ok(());
            }
            match Codec::classify_private1(header.substream_id, &header, body) {
                Some(codec) => stream.codec = codec,
                None if header.is_consistent(pes.payload.len()) => {}
                None => {
                    self.try_announce();
                    return Ok(());
                }
            }
        }
        if stream.codec != Codec::Ac3 {
            self.try_announce();
            return Ok(());
        }
        let body = if stream.es_len == 0 && header.is_consistent(pes.payload.len()) {
            let prefix = usize::from(header.first_access_unit_pointer)
                .saturating_sub(1)
                .min(body.len());
            &body[prefix..]
        } else {
            body
        };
        if !body.is_empty() {
            let stream = &mut self.streams[index];
            stream.push_ac3(body, pts, dts, &self.limits)?;
        }
        self.try_announce();
        Ok(())
    }

    /// Announce every stream whose config has resolved, in first-seen order.
    fn try_announce(&mut self) {
        loop {
            let index = self.announce_cursor;
            let Some(stream) = self.streams.get_mut(index) else {
                break;
            };
            if stream.announced {
                self.announce_cursor += 1;
                continue;
            }
            if stream.resolved && stream.track_id.is_none() {
                self.announce_cursor += 1;
                continue;
            }
            if !stream.config_ready() {
                break;
            }
            let track_id = self.next_track;
            self.next_track = track_id.next();
            let Some(spec) = stream.build_spec(track_id) else {
                break;
            };
            stream.track_id = Some(track_id);
            stream.announced = true;
            self.events.push_back(DemuxEvent::TrackAdded(spec));
            while let Some(sample) = stream.pending.pop_front() {
                self.events.push_back(DemuxEvent::sample(track_id, sample));
            }
            self.announce_cursor += 1;
        }
        self.flush_announced();
    }

    /// Move every announced stream's pending samples into the event queue.
    fn flush_announced(&mut self) {
        for stream in &mut self.streams {
            let Some(track_id) = stream.track_id else {
                continue;
            };
            if !stream.announced {
                continue;
            }
            while let Some(sample) = stream.pending.pop_front() {
                self.events.push_back(DemuxEvent::sample(track_id, sample));
            }
        }
    }

    fn finish_streams(&mut self) -> Result<()> {
        for stream in &mut self.streams {
            stream.finish(&self.limits)?;
        }
        self.try_announce();
        for stream in &mut self.streams {
            if !stream.announced && !stream.config_ready() {
                stream.resolved = true;
            }
        }
        self.try_announce();
        // Samples of a track that never resolved cannot be emitted; the
        // whole-buffer demuxer dropped them too.
        Ok(())
    }
}

/// One elementary stream discovered by its [`StreamKey`].
struct Stream {
    codec: Codec,
    body: Body,
    /// Bytes of the reassembled elementary stream appended so far.
    es_len: u64,
    /// PES-level stamps not yet applied, ordered by stream offset.
    stamps: VecDeque<Stamp>,
    prev_pts_raw: Option<u64>,
    prev_pts_uw: i128,
    prev_dts_raw: Option<u64>,
    prev_dts_uw: i128,
    track_id: Option<TrackId>,
    announced: bool,
    /// No track will ever be produced for this stream.
    resolved: bool,
    pending: VecDeque<Sample>,
    video: VideoTiming,
    ac3_info: Option<Ac3SyncframeInfo>,
    ac3_timescale: Timescale,
    ac3_next_dts: Option<i64>,
    ac3_anchor_wire90: i128,
    ac3_ticks_since_anchor: i64,
}

impl Stream {
    fn new(codec: Codec, body: Body) -> Self {
        Self {
            codec,
            body,
            es_len: 0,
            stamps: VecDeque::new(),
            prev_pts_raw: None,
            prev_pts_uw: 0,
            prev_dts_raw: None,
            prev_dts_uw: 0,
            track_id: None,
            announced: false,
            resolved: false,
            pending: VecDeque::new(),
            video: VideoTiming::new(),
            ac3_info: None,
            ac3_timescale: VIDEO_TIMESCALE,
            ac3_next_dts: None,
            ac3_anchor_wire90: 0,
            ac3_ticks_since_anchor: 0,
        }
    }

    fn push_stamp(&mut self, offset: u64, pts: Option<u64>, dts: Option<u64>) {
        if pts.is_none() && dts.is_none() {
            return;
        }
        let pts = unwrap_stamp(&mut self.prev_pts_raw, &mut self.prev_pts_uw, pts);
        let dts = unwrap_stamp(&mut self.prev_dts_raw, &mut self.prev_dts_uw, dts);
        self.stamps.push_back(Stamp { offset, pts, dts });
    }

    /// The stamp the access unit starting at `offset` carries: the first pending
    /// stamp at or before it, with every later stamp in the same fragment
    /// consumed too (the whole-buffer `assign_stamps` semantics).
    fn take_stamp(&mut self, offset: u64) -> (Option<i128>, Option<i128>) {
        let mut applied: Option<Stamp> = None;
        while let Some(front) = self.stamps.front() {
            if front.offset > offset {
                break;
            }
            let Some(stamp) = self.stamps.pop_front() else {
                break;
            };
            if applied.is_none() {
                applied = Some(stamp);
            }
        }
        applied.map_or((None, None), |stamp| (stamp.pts, stamp.dts))
    }

    fn config_ready(&self) -> bool {
        match &self.body {
            Body::H264(_) => self.video.sps.is_some() && self.video.pps.is_some(),
            Body::Mpeg2(_) => true,
            Body::Ac3(_) => self.ac3_info.is_some(),
            Body::VideoProbe(_) | Body::Skipped => false,
        }
    }

    fn build_spec(&self, track_id: TrackId) -> Option<TrackSpec> {
        match &self.body {
            Body::H264(_) => {
                let config = self.video.avc_config()?;
                Some(TrackSpec::new(
                    track_id,
                    VIDEO_TIMESCALE,
                    CodecConfig::Avc {
                        config,
                        width: 0,
                        height: 0,
                    },
                ))
            }
            Body::Mpeg2(state) => Some(TrackSpec::new(
                track_id,
                VIDEO_TIMESCALE,
                CodecConfig::Mpeg2Video {
                    esds: crate::ts_demux::mpeg2_video_esds(),
                    width: state.width,
                    height: state.height,
                },
            )),
            Body::Ac3(_) => {
                let info = self.ac3_info?;
                Some(TrackSpec::new(
                    track_id,
                    self.ac3_timescale,
                    CodecConfig::Ac3 {
                        config: info.into_dac3(),
                        channel_count: info.channel_count() as u16,
                        sample_rate: info.sample_rate,
                        sample_size: AUDIO_SAMPLE_SIZE_BITS,
                    },
                ))
            }
            Body::VideoProbe(_) | Body::Skipped => None,
        }
    }

    // -- video ---------------------------------------------------------------

    fn push_video(
        &mut self,
        payload: &[u8],
        pts: Option<u64>,
        dts: Option<u64>,
        limits: &PsDemuxLimits,
    ) -> Result<()> {
        let offset = self.es_len;
        self.push_stamp(offset, pts, dts);
        self.es_len += payload.len() as u64;
        if let Body::VideoProbe(probe) = &mut self.body {
            probe.extend_from_slice(payload);
            let decision = classify_video(probe, limits.max_probe_bytes);
            match decision {
                Some(VideoKind::H264) => {
                    let Body::VideoProbe(probe) = core::mem::replace(&mut self.body, Body::Skipped)
                    else {
                        return Ok(());
                    };
                    self.body = Body::H264(H264State::new(&probe)?);
                }
                Some(VideoKind::Mpeg2) => {
                    let Body::VideoProbe(probe) = core::mem::replace(&mut self.body, Body::Skipped)
                    else {
                        return Ok(());
                    };
                    self.body = Body::Mpeg2(Mpeg2State::new(&probe)?);
                }
                None => return Ok(()),
            }
            return self.collect_video_units(limits);
        }
        match &mut self.body {
            Body::H264(state) => state.push(payload)?,
            Body::Mpeg2(state) => state.push(payload),
            _ => return Ok(()),
        }
        self.collect_video_units(limits)
    }

    fn is_h264(&self) -> bool {
        matches!(self.body, Body::H264(_))
    }

    fn collect_video_units(&mut self, limits: &PsDemuxLimits) -> Result<()> {
        loop {
            let unit = match &mut self.body {
                Body::H264(state) => state.pop(),
                Body::Mpeg2(state) => state.pop(),
                _ => None,
            };
            let Some((start, data)) = unit else {
                break;
            };
            let index = self.video.next_index;
            self.video.next_index += 1;
            let (pts, dts) = self.take_stamp(start);
            let h264 = matches!(self.body, Body::H264(_));
            if h264 {
                self.video.scan_parameter_sets(&data);
            }
            let display = match &mut self.body {
                Body::Mpeg2(state) => {
                    state.tracker.observe(&data);
                    state.tracker.state().cloned()
                }
                _ => None,
            };
            let is_sync = if h264 {
                au_is_idr(&data)
            } else {
                mpeg2_is_sync(&data)
            };
            self.video.note_stamp(index, dts);
            self.video.units.push_back(PendingAu {
                index,
                data,
                pts,
                dts,
                is_sync,
                display,
            });
        }
        let h264 = self.is_h264();
        self.video.finalize(h264, false, limits.max_held_samples);
        self.video.drain_samples(h264, &mut self.pending)
    }

    // -- AC-3 ----------------------------------------------------------------

    fn push_ac3(
        &mut self,
        payload: &[u8],
        pts: Option<u64>,
        dts: Option<u64>,
        limits: &PsDemuxLimits,
    ) -> Result<()> {
        let offset = self.es_len;
        self.push_stamp(offset, pts, dts);
        self.es_len += payload.len() as u64;
        let decision = {
            let Body::Ac3(state) = &mut self.body else {
                return Ok(());
            };
            state.push(payload);
            if self.ac3_info.is_none() {
                state.decide()
            } else {
                Decision::Pending
            }
        };
        match decision {
            Decision::Failed => {
                self.resolved = true;
                self.body = Body::Skipped;
                return Ok(());
            }
            Decision::Decided(info) if self.ac3_info.is_none() => {
                self.ac3_info = Some(info);
                self.ac3_timescale = Timescale::try_new(info.sample_rate)?;
            }
            Decision::Pending if self.ac3_info.is_none() => return Ok(()),
            _ => {}
        }
        let mut frames = Vec::new();
        if let Body::Ac3(state) = &mut self.body {
            while let Some(frame) = state.pop_frame() {
                frames.push(frame);
            }
        }
        for (frame_offset, frame) in frames {
            let sample = self.build_ac3_sample(frame_offset, frame);
            if self.pending.len() >= limits.max_held_samples {
                return Err(limit_error("AC-3 samples held before track announcement"));
            }
            self.pending.push_back(sample);
        }
        Ok(())
    }

    fn build_ac3_sample(&mut self, offset: u64, frame: Vec<u8>) -> Sample {
        let (pts, dts) = self.take_stamp(offset);
        let sample_rate = self.ac3_timescale.get();
        let rescale = |ticks_90k: i128| -> i64 {
            ((ticks_90k * i128::from(sample_rate)) / i128::from(VIDEO_TIMESCALE.get())) as i64
        };
        let stamp90 = dts.or(pts);
        let dts = match (stamp90, self.ac3_next_dts) {
            (Some(stamp), Some(running)) => {
                let expected_wire90 = self.ac3_anchor_wire90
                    + (i128::from(self.ac3_ticks_since_anchor) * i128::from(VIDEO_TIMESCALE.get()))
                        / i128::from(sample_rate.max(1));
                let drift = (stamp - expected_wire90).abs();
                if drift > crate::ts_demux::audio_discontinuity_threshold_90k(self.ac3_timescale) {
                    self.ac3_anchor_wire90 = stamp;
                    self.ac3_ticks_since_anchor = 0;
                    rescale(stamp)
                } else {
                    running
                }
            }
            (Some(stamp), None) => {
                self.ac3_anchor_wire90 = stamp;
                self.ac3_ticks_since_anchor = 0;
                rescale(stamp)
            }
            (None, Some(running)) => running,
            (None, None) => {
                let data = bytes::Bytes::from(frame);
                return match Sample::from_raw(
                    data.clone(),
                    None,
                    None,
                    Some(AC3_SAMPLES_PER_SYNCFRAME),
                ) {
                    Ok(sample) => sample,
                    Err(_) => {
                        Sample::timed_raw(data, 0, 0, NonZeroU32::new(AC3_SAMPLES_PER_SYNCFRAME))
                    }
                };
            }
        };
        self.ac3_next_dts = Some(dts + i64::from(AC3_SAMPLES_PER_SYNCFRAME));
        self.ac3_ticks_since_anchor += i64::from(AC3_SAMPLES_PER_SYNCFRAME);
        Sample::timed_raw(frame, dts, dts, NonZeroU32::new(AC3_SAMPLES_PER_SYNCFRAME))
    }

    fn finish(&mut self, limits: &PsDemuxLimits) -> Result<()> {
        match &mut self.body {
            Body::H264(state) => state.finish(),
            Body::Mpeg2(state) => state.finish(),
            _ => {}
        }
        if matches!(self.body, Body::H264(_) | Body::Mpeg2(_)) {
            self.collect_video_units(limits)?;
            if self.config_ready() {
                let h264 = self.is_h264();
                self.video.finalize(h264, true, limits.max_held_samples);
                self.video.drain_samples(h264, &mut self.pending)?;
                self.video.emit_tail(h264, &mut self.pending)?;
            }
        }
        if matches!(self.body, Body::Ac3(_)) && self.ac3_info.is_none() {
            self.resolved = true;
        }
        if !self.config_ready() {
            self.resolved = true;
            self.video.units.clear();
            self.video.ready.clear();
        }
        Ok(())
    }
}

/// One video access unit waiting for its decode time (which needs the frame
/// duration, derived from the first two PES stamps).
struct PendingAu {
    index: u64,
    data: Vec<u8>,
    pts: Option<i128>,
    dts: Option<i128>,
    is_sync: bool,
    display: Option<alloc::sync::Arc<crate::ir::DisplayInfo>>,
}

/// One access unit with its decode/presentation times resolved.
struct ReadyAu {
    data: Vec<u8>,
    dts: i128,
    pts: i128,
    is_sync: bool,
    display: Option<alloc::sync::Arc<crate::ir::DisplayInfo>>,
}

/// The incremental video timing and display state of one elementary stream.
struct VideoTiming {
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    tracker: Option<SpsTracker>,
    units: VecDeque<PendingAu>,
    ready: VecDeque<ReadyAu>,
    next_index: u64,
    first_stamp: Option<(u64, i128)>,
    frame_dur: Option<i128>,
    last_dts: Option<i128>,
}

impl VideoTiming {
    fn new() -> Self {
        Self {
            sps: None,
            pps: None,
            tracker: None,
            units: VecDeque::new(),
            ready: VecDeque::new(),
            next_index: 0,
            first_stamp: None,
            frame_dur: None,
            last_dts: None,
        }
    }

    fn scan_parameter_sets(&mut self, data: &[u8]) {
        for nal in iter_annexb_nals(data) {
            let Some(&header) = nal.first() else {
                continue;
            };
            match header & NAL_TYPE_MASK {
                AVC_NAL_SPS
                    if self.sps.is_none()
                        && parameter_set_is_valid(NalCodec::Avc, ParamKind::Sps, nal) =>
                {
                    self.sps = Some(nal.to_vec());
                }
                AVC_NAL_PPS
                    if self.pps.is_none()
                        && parameter_set_is_valid(NalCodec::Avc, ParamKind::Pps, nal) =>
                {
                    self.pps = Some(nal.to_vec());
                }
                _ => {}
            }
        }
    }

    fn avc_config(&self) -> Option<AVCConfigurationBox> {
        let sps = self.sps.as_ref()?;
        let pps = self.pps.as_ref()?;
        if sps.len() < AVC_SPS_CONFIG_MIN_LEN {
            return None;
        }
        let record = AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: sps[1],
            profile_compatibility: sps[AVC_SPS_CONSTRAINT_FLAGS_OFFSET],
            level_indication: sps[AVC_SPS_LEVEL_IDC_OFFSET],
            length_size_minus_one: broadcast_bitstream::annexb::NAL_LENGTH_SIZE_MINUS_ONE,
            sps: alloc::vec![AvcSps(sps.clone())],
            pps: alloc::vec![AvcPps(pps.clone())],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: alloc::vec::Vec::new(),
        };
        Some(AVCConfigurationBox::new(record))
    }

    /// Derive the constant frame duration from the first two stamped access
    /// units, exactly the span the whole-buffer demuxer averaged.
    fn note_stamp(&mut self, index: u64, dts: Option<i128>) {
        let Some(dts) = dts else {
            return;
        };
        match self.first_stamp {
            None => self.first_stamp = Some((index, dts)),
            Some((first_index, first_dts)) if self.frame_dur.is_none() && index > first_index => {
                let span_index = (index - first_index) as i128;
                let span_dts = dts - first_dts;
                if span_index > 0 && span_dts > 0 {
                    self.frame_dur = Some((span_dts / span_index).max(1));
                }
            }
            Some(_) => {}
        }
    }

    fn seed_tracker(&mut self) {
        if self.tracker.is_some() {
            return;
        }
        let Some(config) = self.avc_config() else {
            return;
        };
        let mut tracker = SpsTracker::new(TrackCodec::Avc);
        tracker.seed_state(DisplayGeometry::from_codec_config(&CodecConfig::Avc {
            config,
            width: 0,
            height: 0,
        }));
        self.tracker = Some(tracker);
    }

    /// Resolve every held access unit's timing once it is knowable, then (when
    /// `force`) everything that is left.
    fn finalize(&mut self, h264: bool, force: bool, held_limit: usize) {
        if self.units.is_empty() {
            return;
        }
        let timing_ready = self.first_stamp.is_some() && self.frame_dur.is_some();
        if !force && !timing_ready && self.units.len() < held_limit {
            return;
        }
        if h264 {
            self.seed_tracker();
            if self.tracker.is_none() {
                return;
            }
        }
        let anchor = self.first_stamp.unwrap_or((0, 0));
        let duration = self.frame_dur.unwrap_or(DEFAULT_FRAME_DURATION);
        while let Some(au) = self.units.pop_front() {
            let dts = au.dts.unwrap_or_else(|| {
                anchor.1 + (i128::from(au.index) - i128::from(anchor.0)) * duration
            });
            let pts = au.pts.unwrap_or(dts);
            let display = if h264 {
                let Some(tracker) = self.tracker.as_mut() else {
                    continue;
                };
                tracker.observe(&au.data);
                tracker.state().cloned()
            } else {
                au.display
            };
            self.ready.push_back(ReadyAu {
                data: au.data,
                dts,
                pts,
                is_sync: au.is_sync,
                display,
            });
        }
    }

    /// Emit every sample whose duration (the gap to the next decode time) is
    /// now known.
    fn drain_samples(&mut self, h264: bool, out: &mut VecDeque<Sample>) -> Result<()> {
        while self.ready.len() >= SAMPLES_PER_DURATION {
            let Some(front) = self.ready.pop_front() else {
                break;
            };
            let next_dts = self.ready.front().map_or(front.dts, |next| next.dts);
            let duration = (next_dts - front.dts).max(0) as u32;
            self.last_dts = Some(front.dts);
            out.push_back(make_video_sample(front, duration, h264)?);
        }
        Ok(())
    }

    /// Emit the final sample: its duration is the previous decode gap, as the
    /// whole-buffer `frame_duration` did.
    fn emit_tail(&mut self, h264: bool, out: &mut VecDeque<Sample>) -> Result<()> {
        let Some(front) = self.ready.pop_front() else {
            return Ok(());
        };
        let duration = match self.last_dts {
            Some(previous) => (front.dts - previous).max(0) as u32,
            None => DEFAULT_FRAME_DURATION as u32,
        };
        out.push_back(make_video_sample(front, duration, h264)?);
        Ok(())
    }
}

fn make_video_sample(au: ReadyAu, duration: u32, h264: bool) -> Result<Sample> {
    let dts = to_ticks(au.dts);
    let pts = to_ticks(au.pts);
    let duration = NonZeroU32::new(duration);
    if h264 {
        let mut sample = Sample::timed_annexb(&au.data, dts, pts, duration, au.is_sync)?;
        sample.display = au.display;
        Ok(sample)
    } else {
        let mut sample = Sample::timed(au.data, dts, pts, duration);
        sample.flags = SampleFlags::new(au.is_sync);
        sample.display = au.display;
        Ok(sample)
    }
}

/// The elementary-stream splitter in force for one stream.
enum Body {
    /// Video bytes accumulated until the codec (H.264 or MPEG-2) is decided.
    VideoProbe(Vec<u8>),
    H264(H264State),
    Mpeg2(Mpeg2State),
    Ac3(Ac3State),
    Skipped,
}

/// Which video codec an elementary stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VideoKind {
    H264,
    Mpeg2,
}

/// Decide a video stream's codec from its buffered prefix.
///
/// A valid MPEG-2 `sequence_header()` (`0x000001B3`, ISO/IEC 13818-2 §6.2.2.1)
/// is structural evidence no H.264 NAL stream produces by chance, and is
/// checked first (C6, #1009). Otherwise a valid in-band H.264 SPS decides it.
/// A stream that shows neither stays undecided until the bound is reached, when
/// it takes the H.264 path (which then recovers no config and produces no
/// track, as the whole-buffer demuxer did).
fn classify_video(probe: &[u8], cap: usize) -> Option<VideoKind> {
    if Mpeg2SeqHeader::find(probe).is_ok() {
        return Some(VideoKind::Mpeg2);
    }
    if has_valid_avc_sps(probe) {
        return Some(VideoKind::H264);
    }
    (probe.len() >= cap).then_some(VideoKind::H264)
}

fn has_valid_avc_sps(data: &[u8]) -> bool {
    iter_annexb_nals(data).any(|nal| {
        nal.first().is_some_and(|header| {
            header & NAL_TYPE_MASK == AVC_NAL_SPS
                && parameter_set_is_valid(NalCodec::Avc, ParamKind::Sps, nal)
        })
    })
}

/// Incremental H.264 Annex B access-unit splitter plus the absolute elementary
/// stream offset of the next unit.
struct H264State {
    splitter: AccessUnitSplitter,
    cursor: Option<u64>,
    /// Bytes from the ES start kept until the first start code is found.
    pre_scan: Vec<u8>,
}

impl H264State {
    fn new(probe: &[u8]) -> Result<Self> {
        let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
        splitter.push(probe)?;
        let cursor = crate::au::first_nal_start(probe).map(|offset| offset as u64);
        let mut pre_scan = Vec::new();
        if cursor.is_none() {
            pre_scan.extend_from_slice(probe);
        }
        Ok(Self {
            splitter,
            cursor,
            pre_scan,
        })
    }

    fn push(&mut self, payload: &[u8]) -> Result<()> {
        self.splitter.push(payload)?;
        if self.cursor.is_none() {
            self.pre_scan.extend_from_slice(payload);
            if let Some(offset) = crate::au::first_nal_start(&self.pre_scan) {
                self.cursor = Some(offset as u64);
                self.pre_scan.clear();
            }
        }
        Ok(())
    }

    fn pop(&mut self) -> Option<(u64, Vec<u8>)> {
        let unit = self.splitter.pop()?;
        let start = self.cursor.unwrap_or(0);
        self.cursor = Some(start.saturating_add(unit.len() as u64));
        Some((start, unit))
    }

    fn finish(&mut self) {
        self.splitter.finish();
    }
}

/// Incremental MPEG-2 picture splitter.
struct Mpeg2State {
    width: u16,
    height: u16,
    tracker: Mpeg2DisplayTracker,
    buf: Vec<u8>,
    /// Absolute ES offset of `buf[0]`.
    base: u64,
    /// Whether a `picture_start_code` has passed: the prologue before the first
    /// picture belongs to the first unit.
    first_picture: bool,
    search_from: usize,
    flushing: bool,
}

impl Mpeg2State {
    fn new(probe: &[u8]) -> Result<Self> {
        let Ok(seq) = Mpeg2SeqHeader::find(probe) else {
            return Err(Error::InvalidInput("ps: MPEG-2 sequence header not found"));
        };
        let mut tracker = Mpeg2DisplayTracker::new();
        if let Ok(geometry) = DisplayGeometry::from_mpeg2_es(probe) {
            tracker.seed_state(Some(geometry));
        }
        Ok(Self {
            width: seq.width,
            height: seq.height,
            tracker,
            buf: probe.to_vec(),
            base: 0,
            first_picture: false,
            search_from: 0,
            flushing: false,
        })
    }

    fn push(&mut self, payload: &[u8]) {
        self.buf.extend_from_slice(payload);
    }

    fn pop(&mut self) -> Option<(u64, Vec<u8>)> {
        if self.flushing {
            if self.buf.is_empty() {
                return None;
            }
            let data = core::mem::take(&mut self.buf);
            let start = self.base;
            self.base = self.base.saturating_add(data.len() as u64);
            return Some((start, data));
        }
        loop {
            let position = find_picture_start(&self.buf, self.search_from)?;
            if !self.first_picture {
                self.first_picture = true;
                self.search_from = position.saturating_add(START_CODE_LEN);
                continue;
            }
            if position == 0 {
                self.search_from = START_CODE_LEN;
                continue;
            }
            let rest = self.buf.split_off(position);
            let data = core::mem::replace(&mut self.buf, rest);
            self.search_from = START_CODE_LEN;
            let start = self.base;
            self.base = self.base.saturating_add(data.len() as u64);
            return Some((start, data));
        }
    }

    fn finish(&mut self) {
        self.flushing = true;
    }
}

fn find_picture_start(buf: &[u8], from: usize) -> Option<usize> {
    let start = from.min(buf.len());
    buf[start..]
        .windows(START_CODE_LEN)
        .position(|window| {
            window[..START_CODE_BYTE_OFFSET] == START_CODE_PREFIX
                && window[START_CODE_BYTE_OFFSET] == MPEG2_PICTURE_START_CODE
        })
        .map(|position| start + position)
}

/// The result of trying to recover an AC-3 substream's configuration.
enum Decision {
    Decided(Ac3SyncframeInfo),
    Pending,
    Failed,
}

/// Incremental AC-3 syncframe splitter.
struct Ac3State {
    buf: Vec<u8>,
    /// Absolute ES offset of `buf[0]`.
    base: u64,
    info: Option<Ac3SyncframeInfo>,
    checked: bool,
}

impl Ac3State {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            base: 0,
            info: None,
            checked: false,
        }
    }

    fn push(&mut self, payload: &[u8]) {
        self.buf.extend_from_slice(payload);
    }

    /// Recover the substream's config from the first syncword, exactly as
    /// `Ac3SyncframeInfo::from_es` did over the whole buffer.
    fn decide(&mut self) -> Decision {
        if self.checked {
            return self.info.map_or(Decision::Failed, Decision::Decided);
        }
        let Some(offset) = find_ac3_syncword(&self.buf) else {
            return Decision::Pending;
        };
        match Ac3SyncframeInfo::from_es(&self.buf[offset..]) {
            Ok(info) => {
                self.info = Some(info);
                self.checked = true;
                Decision::Decided(info)
            }
            Err(Error::BufferTooShort(_)) => Decision::Pending,
            Err(_) => {
                self.checked = true;
                Decision::Failed
            }
        }
    }

    fn pop_frame(&mut self) -> Option<(u64, Vec<u8>)> {
        loop {
            if self.buf.len() < AC3_SYNCWORD.len() {
                return None;
            }
            if !self.buf.starts_with(&AC3_SYNCWORD) {
                self.drop_byte();
                continue;
            }
            match Ac3SyncframeInfo::parse_at(&self.buf, 0) {
                Ok(info) => match info.frame_len_bytes() {
                    Some(len) if len > 0 => {
                        if self.buf.len() < len {
                            return None;
                        }
                        let frame: Vec<u8> = self.buf.drain(..len).collect();
                        let start = self.base;
                        self.base = self.base.saturating_add(len as u64);
                        return Some((start, frame));
                    }
                    _ => self.drop_byte(),
                },
                Err(Error::BufferTooShort(_)) => return None,
                Err(_) => self.drop_byte(),
            }
        }
    }

    fn drop_byte(&mut self) {
        if !self.buf.is_empty() {
            self.buf.drain(..1);
            self.base = self.base.saturating_add(1);
        }
    }
}

fn find_ac3_syncword(buf: &[u8]) -> Option<usize> {
    buf.windows(AC3_SYNCWORD.len())
        .position(|window| window == AC3_SYNCWORD)
}

/// Unwrap one 33-bit timestamp against the stream's previous raw value
/// (delegating to the workspace's one 33-bit wrap implementation).
fn unwrap_ts(previous_unwrapped: i128, previous_raw: u64, raw: u64) -> i128 {
    broadcast_common::clock33::unwrap_delta(previous_unwrapped, previous_raw, raw)
}

/// Unwrap one 33-bit timestamp against the stream's previous raw value.
fn unwrap_stamp(
    previous_raw: &mut Option<u64>,
    previous_unwrapped: &mut i128,
    raw: Option<u64>,
) -> Option<i128> {
    raw.map(|value| match *previous_raw {
        Some(previous) => {
            let unwrapped = unwrap_ts(*previous_unwrapped, previous, value);
            *previous_unwrapped = unwrapped;
            *previous_raw = Some(value);
            unwrapped
        }
        None => {
            *previous_unwrapped = i128::from(value);
            *previous_raw = Some(value);
            i128::from(value)
        }
    })
}

/// Clamp an unwrapped 33-bit-derived timestamp into `Sample::timing`'s range.
fn to_ticks(unwrapped: i128) -> i64 {
    unwrapped.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// True if an Annex B access unit contains an IDR slice NAL (type 5).
fn au_is_idr(access_unit: &[u8]) -> bool {
    iter_annexb_nals(access_unit).any(|nal| {
        nal.first()
            .is_some_and(|header| header & NAL_TYPE_MASK == AVC_NAL_IDR)
    })
}

fn limit_error(what: &'static str) -> Error {
    Error::InvalidInput(what)
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    const WRAP: u64 = broadcast_common::clock33::WRAP_33BIT;

    /// PS stamps straddling 2^33 (ISO/IEC 13818-1 §2.4.3.7): PTS 2^33-90000,
    /// then 90000 (one wrap, +180000 ticks), then a B-frame style backward
    /// dip back across the wrap (2^33-45000) must unroll monotonically /
    /// bidirectionally, never jump by ~2^33.
    #[test]
    fn assign_stamps_unrolls_across_the_33bit_wrap() {
        let mut stream = Stream::new(Codec::Video, Body::VideoProbe(Vec::new()));
        stream.push_stamp(0, Some(WRAP - 90_000), Some(WRAP - 90_000));
        stream.push_stamp(10, Some(90_000), Some(90_000));
        stream.push_stamp(20, Some(WRAP - 45_000), Some(WRAP - 45_000));
        let first = stream.take_stamp(0);
        let second = stream.take_stamp(10);
        let third = stream.take_stamp(20);
        assert_eq!(first.0, Some(i128::from(WRAP - 90_000)));
        assert_eq!(second.0, Some(i128::from(WRAP + 90_000)));
        assert_eq!(third.0, Some(i128::from(WRAP - 45_000)));
        assert_eq!(second.1, Some(i128::from(WRAP + 90_000)));
    }

    /// When a second PES stamp falls in the same access unit only the first
    /// applies. A PTS-only first stamp makes the `&&` visible: the second
    /// stamp's PTS must not replace it just because the DTS is still unknown.
    #[test]
    fn assign_stamps_keeps_the_first_stamp_of_an_access_unit() {
        let mut stream = Stream::new(Codec::Video, Body::VideoProbe(Vec::new()));
        stream.push_stamp(0, Some(180_000), None);
        stream.push_stamp(0, Some(270_000), Some(270_000));
        assert_eq!(
            stream.take_stamp(0),
            (Some(180_000), None),
            "the first stamp of the access unit wins"
        );

        stream.push_stamp(0, Some(1_000), Some(900));
        stream.push_stamp(0, Some(2_000), Some(1_900));
        assert_eq!(
            stream.take_stamp(0),
            (Some(1_000), Some(900)),
            "a fully stamped access unit keeps its first stamp"
        );
    }
}

#[cfg(test)]
mod h264_config_tests {
    use super::*;

    fn annexb(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&[0, 0, 1]);
            out.extend_from_slice(nal);
        }
        out
    }

    /// Drive the stage over `data` and return its events.
    fn drive(data: &[u8]) -> Vec<DemuxEvent> {
        let mut demux = StreamingPsDemux::new();
        demux
            .feed(data, broadcast_common::stage::Timestamp::ZERO)
            .expect("feed");
        demux.finish().expect("finish");
        let mut events = Vec::new();
        while let Some(event) = demux.poll() {
            events.push(event);
        }
        events
    }

    /// The real SPS, PPS and first coded slice of the committed PS fixture.
    fn fixture_parts() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let data = test_fixtures::read("ps/h264-ac3.ps");
        let events = drive(&data);
        let (track_id, config) = events
            .iter()
            .find_map(|event| match event {
                DemuxEvent::TrackAdded(spec) => match &spec.config {
                    CodecConfig::Avc { config, .. } => Some((spec.track_id, config.clone())),
                    _ => None,
                },
                _ => None,
            })
            .expect("track 0 must be AVC");
        let sps = config.config.sps[0].0.clone();
        let pps = config.config.pps[0].0.clone();
        let sample = events
            .iter()
            .find_map(|event| match event {
                DemuxEvent::Sample {
                    track_id: id,
                    sample,
                } if *id == track_id => Some(sample),
                _ => None,
            })
            .expect("the AVC track carries samples")
            .data
            .clone();
        let mut off = 0usize;
        let mut slice = Vec::new();
        while off + 4 <= sample.len() {
            let n = u32::from_be_bytes([
                sample[off],
                sample[off + 1],
                sample[off + 2],
                sample[off + 3],
            ]) as usize;
            off += 4;
            if off + n > sample.len() {
                break;
            }
            let nal = &sample[off..off + n];
            off += n;
            let nal_type = nal[0] & NAL_TYPE_MASK;
            if nal_type == 1 || nal_type == 5 {
                slice = nal.to_vec();
                break;
            }
        }
        assert!(
            !slice.is_empty(),
            "the fixture's first sample carries a coded slice"
        );
        (sps, pps, slice)
    }

    fn built_config(nals: &[&[u8]]) -> Option<(Vec<u8>, Vec<u8>)> {
        let mut timing = VideoTiming::new();
        timing.scan_parameter_sets(&annexb(nals));
        let config = timing.avc_config()?;
        Some((
            config.config.sps[0].0.clone(),
            config.config.pps[0].0.clone(),
        ))
    }

    /// A damaged parameter set (a set `forbidden_zero_bit`) must never reach
    /// the `avcC`, in either position, and a stream with only damaged sets is
    /// declined rather than built from garbage (issue #1171).
    #[test]
    fn damaged_parameter_sets_are_skipped() {
        let (sps, pps, slice) = fixture_parts();
        let garbage_sps = [0xE7u8, 0x01, 0x02, 0x03, 0xFF];
        let garbage_pps = [0xE8u8, 0x01, 0x02, 0x03, 0xFF];

        let (got_sps, got_pps) = built_config(&[&garbage_sps, &sps, &garbage_pps, &pps, &slice])
            .expect("the real SPS/PPS are present");
        assert_eq!(
            got_sps, sps,
            "the damaged SPS before the real one is skipped"
        );
        assert_eq!(
            got_pps, pps,
            "the damaged PPS before the real one is skipped"
        );
        assert_eq!(got_sps[0], 0x67, "the real SPS NAL header");

        let (got_sps, got_pps) = built_config(&[&sps, &garbage_sps, &pps, &garbage_pps, &slice])
            .expect("the real SPS/PPS are present");
        assert_eq!(
            got_sps, sps,
            "a damaged SPS after the real one must not replace it"
        );
        assert_eq!(
            got_pps, pps,
            "a damaged PPS after the real one must not replace it"
        );

        assert!(
            built_config(&[&garbage_sps, &garbage_pps, &slice]).is_none(),
            "a stream with only damaged parameter sets must not build a config"
        );
    }

    /// The first valid SPS/PPS is kept: a later valid set must not replace it.
    #[test]
    fn the_first_valid_parameter_set_wins() {
        let (sps, pps, slice) = fixture_parts();

        let mut other_sps = sps.clone();
        other_sps[3] = if sps[3] == 0x1E { 0x1F } else { 0x1E };
        let mut other_pps = pps.clone();
        let last = other_pps.len() - 1;
        other_pps[last] = if other_pps[last] == 0x7F { 0x7E } else { 0x7F };
        assert!(parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Sps,
            &other_sps
        ));
        assert!(parameter_set_is_valid(
            NalCodec::Avc,
            ParamKind::Pps,
            &other_pps
        ));
        assert_ne!(other_sps, sps);
        assert_ne!(other_pps, pps);

        let (got_sps, got_pps) = built_config(&[&sps, &other_sps, &pps, &other_pps, &slice])
            .expect("a valid SPS/PPS is present");
        assert_eq!(got_sps, sps, "the first valid SPS is kept");
        assert_eq!(got_pps, pps, "the first valid PPS is kept");
    }
}

#[cfg(test)]
mod interpolate_tests {
    use super::*;

    fn pending(index: u64, dts: Option<i128>) -> PendingAu {
        PendingAu {
            index,
            data: Vec::new(),
            pts: dts,
            dts,
            is_sync: false,
            display: None,
        }
    }

    /// Audit #3570: the constant frame duration divides the stamp span by the
    /// **index** span, and the interpolation walks from the anchor by index
    /// distance. Stamps on access units 2 and 6 (not 0) make both signs visible:
    /// `i1 - i0` is 4 (not 8) and `i - anchor_idx` is negative before the anchor.
    #[test]
    fn stamped_frame_duration_uses_the_index_span_from_the_first_stamp() {
        let mut timing = VideoTiming::new();
        for index in 0..7 {
            timing.units.push_back(pending(index, None));
        }
        timing.note_stamp(2, Some(90_000));
        timing.note_stamp(6, Some(90_000 + 4 * 3_600));
        assert_eq!(timing.frame_dur, Some(3_600));

        timing.finalize(false, true, 4096);
        let dts: Vec<i128> = timing.ready.iter().map(|au| au.dts).collect();
        assert_eq!(dts[0], 90_000 - 2 * 3_600);
        assert_eq!(dts[1], 90_000 - 3_600);
        assert_eq!(dts[2], 90_000);
        assert_eq!(dts[4], 90_000 + 2 * 3_600);
        assert_eq!(dts[6], 90_000 + 4 * 3_600);
    }

    /// Audit #3570: a backwards or repeated stamp pair yields no usable frame
    /// period and must fall back to the 3 600-tick default (25 fps at 90 kHz).
    #[test]
    fn non_increasing_stamps_fall_back_to_the_default_frame_duration() {
        let mut descending = VideoTiming::new();
        for index in 0..3 {
            descending.units.push_back(pending(index, None));
        }
        descending.note_stamp(0, Some(90_000));
        descending.note_stamp(2, Some(90_000 - 7_200));
        assert_eq!(descending.frame_dur, None);
        descending.finalize(false, true, 4096);
        let dts: Vec<i128> = descending.ready.iter().map(|au| au.dts).collect();
        assert_eq!(dts[0], 90_000);
        assert_eq!(dts[1], 90_000 + 3_600);

        let mut repeated = VideoTiming::new();
        for index in 0..3 {
            repeated.units.push_back(pending(index, None));
        }
        repeated.note_stamp(0, Some(90_000));
        repeated.note_stamp(2, Some(90_000));
        assert_eq!(repeated.frame_dur, None);
        repeated.finalize(false, true, 4096);
        let dts: Vec<i128> = repeated.ready.iter().map(|au| au.dts).collect();
        assert_eq!(dts[2], 90_000 + 2 * 3_600);
    }

    const AC3_FIXTURE: &str = "ps/ffmpeg-mpeg2video-2xac3.a0.ac3";

    /// A real AC-3 elementary stream whose second syncframe carries `stamp90`,
    /// the first carrying 90 000. The fixture is 44.1 kHz mono, so one 1 536-
    /// sample syncframe is 3 134.7 ticks and the frame-exact running clock sits
    /// 1 536 track ticks per frame after the anchor.
    fn ac3_stream(stamp90: u64) -> (Stream, Vec<(usize, usize)>) {
        let data = std::fs::read(test_fixtures::path(AC3_FIXTURE))
            .expect("real AC-3 elementary stream fixture");
        let frames = crate::ac3_syncframe::split_ac3_syncframe_ranges(&data, true);
        assert!(frames.len() >= 3, "fixture carries several syncframes");
        let info = Ac3SyncframeInfo::from_es(&data).expect("AC-3 syncframe info");
        let mut stream = Stream::new(Codec::Ac3, Body::Ac3(Ac3State::new()));
        stream.ac3_info = Some(info);
        stream.ac3_timescale = Timescale::try_new(info.sample_rate).expect("sample rate");
        stream.push_stamp(frames[0].0 as u64, Some(90_000), Some(90_000));
        stream.push_stamp(frames[1].0 as u64, Some(stamp90), Some(stamp90));
        (stream, frames)
    }

    fn ac3_dts(stamp90: u64) -> Vec<i64> {
        let (mut stream, frames) = ac3_stream(stamp90);
        let mut out = Vec::new();
        for &(start, _) in frames.iter().take(3) {
            let sample = stream.build_ac3_sample(start as u64, Vec::new());
            out.push(
                sample
                    .timing
                    .and_then(|timing| timing.dts())
                    .expect("timed AC-3 sample"),
            );
        }
        out
    }

    /// The first stamp anchors at 44 100 track ticks (90 000/2 at 44.1 kHz);
    /// each following syncframe advances by its intrinsic 1 536 samples.
    #[test]
    fn ac3_stamp_exactly_on_the_frame_grid_keeps_the_running_clock() {
        let dts = ac3_dts(93_134);
        assert_eq!(dts[0], 44_100);
        assert_eq!(dts[1], 45_636);
        assert_eq!(dts[2], 47_172);
    }

    /// Audit #3571: a stamp exactly `audio_discontinuity_threshold_90k` (1 800
    /// ticks) off the predicted wire position is *not* a discontinuity.
    #[test]
    fn ac3_stamp_at_the_threshold_is_not_a_discontinuity() {
        let dts = ac3_dts(93_134 + 1_800);
        assert_eq!(
            dts[1], 45_636,
            "within the threshold the running clock wins"
        );
        assert_eq!(dts[2], 47_172);
    }

    /// One tick past the threshold re-anchors from the wire stamp: 94 935 ticks
    /// rescales to 46 518 at 44.1 kHz, and the clock continues from there.
    #[test]
    fn ac3_stamp_one_tick_past_the_threshold_reanchors() {
        let dts = ac3_dts(93_134 + 1_801);
        assert_eq!(dts[1], 46_518);
        assert_eq!(dts[2], 48_054);
    }

    /// The same drift with the opposite sign must also re-anchor (the drift is
    /// an absolute distance): 91 333 ticks rescale to 44 753.
    #[test]
    fn ac3_negative_drift_past_the_threshold_reanchors() {
        let dts = ac3_dts(93_134 - 1_801);
        assert_eq!(dts[1], 44_753);
        assert_eq!(dts[2], 46_289);
    }
}

#[cfg(test)]
mod private1_tests {
    use super::*;

    /// Drive the stage over `data` and return its events.
    fn drive(data: &[u8]) -> Vec<DemuxEvent> {
        let mut demux = StreamingPsDemux::new();
        demux
            .feed(data, broadcast_common::stage::Timestamp::ZERO)
            .expect("feed");
        demux.finish().expect("finish");
        let mut events = Vec::new();
        while let Some(event) = demux.poll() {
            events.push(event);
        }
        events
    }

    fn header(substream_id: u8, number_of_frames: u8, pointer: u16) -> Private1Header {
        Private1Header {
            substream_id,
            number_of_frames,
            first_access_unit_pointer: pointer,
        }
    }

    /// The header's four bytes are `substream_id`, `number_of_frames` and a
    /// **big-endian** 16-bit pointer: reading the pointer's high byte twice
    /// equals the right value only while it is zero.
    #[test]
    fn parse_reads_the_pointer_big_endian() {
        let (parsed, rest) = Private1Header::parse(&[0x80, 1, 0x01, 0x23]).expect("header");
        assert_eq!(parsed.substream_id, 0x80);
        assert_eq!(parsed.number_of_frames, 1);
        assert_eq!(parsed.first_access_unit_pointer, 0x0123);
        assert!(rest.is_empty(), "a header-only payload has no body");
    }

    /// A payload that cannot hold the whole header is refused, never indexed
    /// past its end.
    #[test]
    fn a_payload_shorter_than_the_header_is_refused() {
        assert!(Private1Header::parse(&[]).is_none());
        assert!(Private1Header::parse(&[0x80]).is_none());
        assert!(Private1Header::parse(&[0x80, 1]).is_none());
        assert!(Private1Header::parse(&[0x80, 1, 2]).is_none());
    }

    /// The consistency rule (ISO/IEC 13818-1 §2.4.3.7 as the DVD/ATSC private
    /// profiles use it): a header that claims frames must point at one, and a
    /// pointer may address one byte past the body (an access unit that starts
    /// at its very end) but no further.
    #[test]
    fn is_consistent_truth_table() {
        for (frames, pointer, body_len, want) in [
            (1u8, 0u16, 0usize, false),
            (0, 0, 0, true),
            (1, 1, 0, true),
            (1, 2, 0, false),
            (0, 1, 0, true),
            (0, 0, 5, true),
            (1, 6, 5, true),
            (1, 7, 5, false),
            (0, 6, 5, true),
            (0, 7, 5, false),
        ] {
            assert_eq!(
                header(0x80, frames, pointer).is_consistent(body_len + PRIVATE1_HEADER_LEN),
                want,
                "frames={frames} pointer={pointer} body_len={body_len}"
            );
        }
    }

    /// A payload shorter than the header does not underflow the body length.
    #[test]
    fn is_consistent_does_not_underflow_a_short_payload() {
        assert!(header(0x80, 0, 0).is_consistent(0));
        assert!(header(0x80, 0, 0).is_consistent(3));
        assert!(!header(0x80, 1, 0).is_consistent(3));
    }

    /// The AC-3 probe: a payload whose probed bytes are not the 2-byte
    /// syncword is not AC-3, whatever its length, and a payload that is
    /// exactly the syncword is.
    #[test]
    fn classify_private1_probes_the_ac3_syncword() {
        assert_eq!(
            Codec::classify_private1(0x80, &header(0x80, 1, 1), &[0x12, 0x34, 0x56]),
            None
        );
        assert_eq!(
            Codec::classify_private1(0x80, &header(0x80, 1, 1), &AC3_SYNCWORD),
            Some(Codec::Ac3)
        );
        assert_eq!(
            Codec::classify_private1(0x80, &header(0x80, 1, 1), &[0x0B]),
            None
        );
    }

    /// A `picture_start_code` in the final three bytes is not a picture: the
    /// scan must not index past the end of the stream.
    #[test]
    fn split_mpeg2_pictures_handles_a_start_code_at_the_tail() {
        fn ranges(data: &[u8]) -> Vec<(u64, usize)> {
            let mut state = Mpeg2State {
                width: 0,
                height: 0,
                tracker: Mpeg2DisplayTracker::new(),
                buf: data.to_vec(),
                base: 0,
                first_picture: false,
                search_from: 0,
                flushing: false,
            };
            let mut out = Vec::new();
            while let Some((start, unit)) = state.pop() {
                out.push((start, unit.len()));
            }
            state.finish();
            while let Some((start, unit)) = state.pop() {
                out.push((start, unit.len()));
            }
            out
        }

        let data = [0u8, 0, 1, 0xB3, 1, 2, 0, 0, 1];
        assert_eq!(ranges(&data), alloc::vec![(0, data.len())]);

        let data = [0u8, 0, 1, 0x00, 1, 2, 0, 0, 1, 0x00];
        assert_eq!(ranges(&data), alloc::vec![(0, 6), (6, data.len() - 6)]);
    }

    /// A complete, valid-looking AC-3 frame in the prefix the pointer skips
    /// past is *not* part of the track: the first packet of a substream
    /// starts the elementary stream at the access unit the pointer names.
    #[test]
    fn a_complete_frame_before_the_first_access_unit_is_not_emitted() {
        let es = std::fs::read(test_fixtures::path("ps/ffmpeg-mpeg2video-2xac3.a0.ac3"))
            .expect("real AC-3 elementary stream fixture");
        let frames = crate::ac3_syncframe::split_ac3_syncframes(&es);
        assert!(frames.len() >= 2, "fixture carries several syncframes");
        let (first, second) = (frames[0], frames[1]);

        let mut body = first.to_vec();
        body.extend_from_slice(second);
        let pointer = u16::try_from(first.len() + 1).expect("a small frame");
        let mut payload = alloc::vec![0x80u8, 1];
        payload.extend_from_slice(&pointer.to_be_bytes());
        payload.extend_from_slice(&body);

        let pack_header = &std::fs::read(test_fixtures::path("ps/ffmpeg-mpeg2video-2xac3.ps"))
            .expect("real PS fixture")[..14];
        let mut ps = pack_header.to_vec();
        ps.extend_from_slice(&[0, 0, 1, STREAM_ID_PRIVATE_1]);
        let pes_data_len = 3 + payload.len();
        ps.extend_from_slice(
            &u16::try_from(pes_data_len)
                .expect("a small PES")
                .to_be_bytes(),
        );
        ps.extend_from_slice(&[0x80, 0x00, 0x00]);
        ps.extend_from_slice(&payload);

        let events = drive(&ps);
        let track_id = events
            .iter()
            .find_map(|event| match event {
                DemuxEvent::TrackAdded(spec) if matches!(spec.config, CodecConfig::Ac3 { .. }) => {
                    Some(spec.track_id)
                }
                _ => None,
            })
            .expect("the AC-3 substream");
        let samples: Vec<&Sample> = events
            .iter()
            .filter_map(|event| match event {
                DemuxEvent::Sample {
                    track_id: id,
                    sample,
                } if *id == track_id => Some(sample),
                _ => None,
            })
            .collect();
        assert_eq!(
            samples.len(),
            1,
            "only the access unit the pointer names is emitted"
        );
        assert_eq!(
            samples[0].data.as_ref(),
            second,
            "the sample is the second frame, not the prefix frame"
        );
    }
}
