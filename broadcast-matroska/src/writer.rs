//! The sans-IO Matroska/WebM writer.
//!
//! [`Writer::new`] takes the caller's `Info` and `Tracks` elements and emits
//! the EBML header, the `Segment` with the RFC 8794 §6.2 unknown size, a
//! `SeekHead`, `Info` and `Tracks`. [`Writer::push_block`] appends one Block at
//! a time, cutting a new `Cluster` by the caller's [`Layout`] span, at every
//! video keyframe and whenever a signed 16-bit relative timestamp would leave
//! its range (RFC 9559 §12). [`Writer::finish`] flushes the open `Cluster`,
//! writes `Cues` (one point per video keyframe, RFC 9559 §20) and patches
//! `Info.Duration` at the offset the header wrote it.
//!
//! Every write is returned, never performed: [`Write::Append`] appends bytes
//! and [`Write::At`] replaces a fixed range already written, so the caller owns
//! the output buffer and its length is a whole-stream fact the writer never
//! holds.
//!
//! # Spec citations
//!
//! - EBML headers and the unknown size: RFC 8794 §4, §6.2, §11.2
//!   (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`).
//! - Segment, SeekHead, Info, Tracks, Cluster, SimpleBlock and Cues: RFC 9559
//!   §5, §6, §11, §12, §20
//!   (`private/specs-verified/ietf/specs/rfc9559_matroska.txt`).

use alloc::vec;
use alloc::vec::Vec;

use broadcast_core::Serialize;

use crate::ebml::leaf::{BinaryElement, FloatElement, TextElement, UnsignedElement};
use crate::ebml::scalar::{Binary, Float, IntegerWidth, Text, Unsigned};
use crate::ebml::{ElementHeader, ElementId, VintSize};
use crate::error::{Error, Result};
use crate::matroska::ids;
use crate::matroska::types::TrackType;
use crate::matroska::{Audio, Element, Info, Tracks, Video};

/// The EBML header's `DocType` for Matroska (RFC 9559 §3).
const DOC_TYPE: &str = "matroska";
/// `EBMLVersion` this writer emits (RFC 8794 §11.2.2).
const EBML_VERSION: u64 = 1;
/// `EBMLReadVersion` this writer emits (§11.2.3).
const EBML_READ_VERSION: u64 = 1;
/// `EBMLMaxIDLength` this writer emits (§11.2.4).
const EBML_MAX_ID_LENGTH: u64 = 4;
/// `EBMLMaxSizeLength` this writer emits (§11.2.5).
const EBML_MAX_SIZE_LENGTH: u64 = 8;
/// `DocTypeVersion` this writer emits (RFC 9559 §3).
const DOC_TYPE_VERSION: u64 = 4;
/// `DocTypeReadVersion` this writer emits (RFC 9559 §3).
const DOC_TYPE_READ_VERSION: u64 = 2;
/// The `MuxingApp`/`WritingApp` this writer emits when the caller names none.
const DEFAULT_APP: &str = "transmux";
/// Width of a `SeekPosition`, fixed so a `SeekHead`'s own length does not
/// depend on the positions it carries.
const SEEK_POSITION_WIDTH: usize = 4;
/// The unknown Element Data Size, at its widest width (RFC 8794 §6.2).
const UNKNOWN_SIZE: VintSize = VintSize::unknown_max();
/// `SimpleBlock` flags octet width (RFC 9559 §12).
const FLAG_OCTETS: usize = 1;
/// Octets in a Block's relative timestamp (RFC 9559 §12).
const TIMESTAMP_OCTETS: usize = 2;
/// The keyframe bit of a `SimpleBlock`'s flags octet (RFC 9559 §12).
const FLAG_KEYFRAME: u8 = 0x80;
/// The invisible bit.
const FLAG_INVISIBLE: u8 = 0x08;
/// The discardable bit.
const FLAG_DISCARDABLE: u8 = 0x01;

/// How the writer cuts Clusters and patches `Info.Duration`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    cluster_span: core::time::Duration,
    duration_ticks: Option<f64>,
}

impl Layout {
    /// Cut a new Cluster when its span reaches `cluster_span` (in the Segment's
    /// `TimestampScale` ticks). A video keyframe always cuts, and so does a
    /// relative timestamp that would overflow the `SimpleBlock` `i16`
    /// (RFC 9559 §12) — both are format invariants, not caller policy.
    #[must_use]
    pub const fn new(cluster_span: core::time::Duration) -> Self {
        Self {
            cluster_span,
            duration_ticks: None,
        }
    }

    /// Use `duration_ticks` for `Info.Duration` when no pushed Block states a
    /// duration. Without it the writer derives the duration from the Blocks.
    #[must_use]
    pub const fn with_duration_ticks(mut self, duration_ticks: Option<f64>) -> Self {
        self.duration_ticks = duration_ticks;
        self
    }

    /// The caller's Cluster span.
    #[must_use]
    pub const fn cluster_span(&self) -> core::time::Duration {
        self.cluster_span
    }
}

/// One Block's flags (RFC 9559 §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockFlags {
    /// The `SimpleBlock` keyframe bit.
    pub keyframe: bool,
    /// The `SimpleBlock` discardable bit.
    pub discardable: bool,
    /// The `SimpleBlock` invisible bit.
    pub invisible: bool,
}

impl BlockFlags {
    /// Build the three flags.
    #[must_use]
    pub const fn new(keyframe: bool, discardable: bool, invisible: bool) -> Self {
        Self {
            keyframe,
            discardable,
            invisible,
        }
    }

    /// The keyframe flag alone.
    #[must_use]
    pub const fn keyframe(keyframe: bool) -> Self {
        Self {
            keyframe,
            discardable: false,
            invisible: false,
        }
    }
}

/// One write the caller must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Write {
    /// Append these bytes at the end of the output.
    Append(Vec<u8>),
    /// Replace the bytes at `offset` with these (the same length).
    At {
        /// Absolute output offset.
        offset: u64,
        /// Replacement bytes.
        bytes: Vec<u8>,
    },
}

/// The `Info` fields this writer's builder accepts (RFC 9559 §5.1.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfoSpec<'a> {
    /// `TimestampScale` in nanoseconds per Matroska Tick (§11).
    pub timestamp_scale_ns: u64,
    /// `Duration` in Matroska Ticks.
    pub duration_ticks: f64,
    /// `MuxingApp`.
    pub muxing_app: &'a str,
    /// `WritingApp`.
    pub writing_app: &'a str,
}

impl<'a> InfoSpec<'a> {
    /// A spec carrying the writer's default application name.
    #[must_use]
    pub const fn new(timestamp_scale_ns: u64, duration_ticks: f64) -> Self {
        Self {
            timestamp_scale_ns,
            duration_ticks,
            muxing_app: DEFAULT_APP,
            writing_app: DEFAULT_APP,
        }
    }
}

/// One track's `TrackEntry` fields this writer's builder accepts
/// (RFC 9559 §5.1.4.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackSpec<'a> {
    /// `TrackNumber`, 1-based.
    pub track_number: u64,
    /// `TrackUID`.
    pub track_uid: u64,
    /// `TrackType` (1 video, 2 audio, 33 metadata, …).
    pub track_type: u64,
    /// `CodecID`.
    pub codec_id: &'a str,
    /// `CodecPrivate`.
    pub codec_private: Option<&'a [u8]>,
    /// `DefaultDuration` in nanoseconds.
    pub default_duration_ns: Option<u64>,
    /// `Video` (`PixelWidth`, `PixelHeight`).
    pub video: Option<(u64, u64)>,
    /// `Audio` (`Channels`, `SamplingFrequency`).
    pub audio: Option<(u64, f64)>,
}

/// Build an `Info` element with `TimestampScale`, `Duration`, `MuxingApp` and
/// `WritingApp`.
pub fn info_element<'a>(spec: &InfoSpec<'a>) -> Result<Info<'a>> {
    let children = vec![
        unsigned_element(ids::TIMESTAMP_SCALE, spec.timestamp_scale_ns)?,
        float_element(ids::DURATION, spec.duration_ticks)?,
        text_element(ids::MUXING_APP, spec.muxing_app)?,
        text_element(ids::WRITING_APP, spec.writing_app)?,
    ];
    Ok(Info::from_parts(master_header(ids::INFO)?, children))
}

/// Build a `Tracks` element from `specs`, in order.
pub fn tracks_element<'a>(specs: &'a [TrackSpec<'a>]) -> Result<Tracks<'a>> {
    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        entries.push(Element::TrackEntry(track_entry(spec)?));
    }
    Ok(Tracks::from_parts(master_header(ids::TRACKS)?, entries))
}

fn track_entry<'a>(spec: &'a TrackSpec<'a>) -> Result<crate::matroska::TrackEntry<'a>> {
    let mut children = vec![
        unsigned_element(ids::TRACK_NUMBER, spec.track_number)?,
        unsigned_element(ids::TRACK_UID, spec.track_uid)?,
        unsigned_element(ids::TRACK_TYPE, spec.track_type)?,
        text_element(ids::CODEC_ID, spec.codec_id)?,
    ];
    if let Some(ns) = spec.default_duration_ns {
        children.push(unsigned_element(ids::DEFAULT_DURATION, ns)?);
    }
    if let Some((width, height)) = spec.video {
        let video_children = vec![
            unsigned_element(ids::PIXEL_WIDTH, width)?,
            unsigned_element(ids::PIXEL_HEIGHT, height)?,
        ];
        children.push(Element::Video(Video::from_parts(
            master_header(ids::VIDEO)?,
            video_children,
        )));
    }
    if let Some((channels, sample_rate)) = spec.audio {
        let audio_children = vec![
            unsigned_element(ids::CHANNELS, channels)?,
            float_element(ids::SAMPLING_FREQUENCY, sample_rate)?,
        ];
        children.push(Element::Audio(Audio::from_parts(
            master_header(ids::AUDIO)?,
            audio_children,
        )));
    }
    if let Some(private) = spec.codec_private {
        children.push(binary_element(ids::CODEC_PRIVATE, private)?);
    }
    Ok(crate::matroska::TrackEntry::from_parts(
        master_header(ids::TRACK_ENTRY)?,
        children,
    ))
}

fn element_header(id: u32) -> Result<ElementHeader> {
    ElementHeader::with_payload_len(ElementId::try_from_raw(id)?, 0)
}

fn master_header(id: u32) -> Result<ElementHeader> {
    element_header(id)
}

fn unsigned_element(id: u32, value: u64) -> Result<Element<'static>> {
    let value = Unsigned::new(value);
    let header = sized_header(id, value.serialized_len())?;
    Ok(Element::Unsigned(UnsignedElement::new(header, value)))
}

fn float_element(id: u32, value: f64) -> Result<Element<'static>> {
    let value = Float::new(value);
    let header = sized_header(id, value.serialized_len())?;
    Ok(Element::Float(FloatElement::new(header, value)))
}

fn text_element(id: u32, value: &str) -> Result<Element<'_>> {
    let value = Text::new(value.as_bytes());
    let header = sized_header(id, value.serialized_len())?;
    Ok(Element::Text(TextElement::new(header, value)))
}

fn binary_element(id: u32, value: &[u8]) -> Result<Element<'_>> {
    let value = Binary::new(value);
    let header = sized_header(id, value.serialized_len())?;
    Ok(Element::Binary(BinaryElement::new(header, value)))
}

fn sized_header(id: u32, payload_len: usize) -> Result<ElementHeader> {
    ElementHeader::with_payload_len(
        ElementId::try_from_raw(id)?,
        u64::try_from(payload_len).map_err(|_| Error::SizeOverflow)?,
    )
}

/// One Block waiting to be placed in a `Cluster`.
#[derive(Debug, Clone)]
struct Pending {
    track_index: usize,
    track_number: u64,
    pts: i64,
    keyframe: bool,
    discardable: bool,
    invisible: bool,
    is_video: bool,
    data: Vec<u8>,
}

/// One `CuePoint` source: a video keyframe's time and the byte offset
/// (relative to the Segment payload start) of its `Cluster`.
#[derive(Debug, Clone, Copy)]
struct Cue {
    time: i64,
    track: u64,
    offset: u64,
}

/// The Matroska writer. See the [module docs](self).
#[derive(Debug)]
pub struct Writer {
    layout: Layout,
    span_ticks: i64,
    has_video: bool,
    tracks: Vec<(u64, bool)>,
    pending: Vec<Option<Pending>>,
    cluster: Vec<Pending>,
    cluster_start: i64,
    cues: Vec<Cue>,
    payload_written: u64,
    duration_ticks: i64,
    info_duration_offset: u64,
    head: Vec<Write>,
    started: bool,
}

impl Writer {
    /// Build a writer from the caller's `Info` and `Tracks`.
    ///
    /// `Info` must carry an eight-octet `Duration`; the writer records its
    /// payload offset and patches it in [`Writer::finish`].
    pub fn new(layout: Layout, info: &Info<'_>, tracks: &Tracks<'_>) -> Result<Self> {
        let ebml = ebml_header()?;
        let info_bytes = info.try_to_bytes()?;
        let tracks_bytes = tracks.try_to_bytes()?;
        let tracks_bytes_len = tracks_bytes.len();
        let duration_in_info = duration_payload_offset(info)?;
        let placeholder = seek_head(0, 0)?;
        let pos_info = u64::try_from(placeholder.len()).map_err(|_| Error::SizeOverflow)?;
        let pos_tracks = pos_info
            .checked_add(u64::try_from(info_bytes.len()).map_err(|_| Error::SizeOverflow)?)
            .ok_or(Error::SizeOverflow)?;
        let seek = seek_head(pos_info, pos_tracks)?;
        let mut segment = Vec::with_capacity(ElementId::try_from_raw(ids::SEGMENT)?.octets());
        append_element(&mut segment, ids::SEGMENT, None)?;

        let info_offset = u64::try_from(ebml.len())
            .map_err(|_| Error::SizeOverflow)?
            .checked_add(u64::try_from(segment.len()).map_err(|_| Error::SizeOverflow)?)
            .and_then(|offset| offset.checked_add(u64::try_from(seek.len()).ok()?))
            .ok_or(Error::SizeOverflow)?;

        let mut metadata = Vec::new();
        for entry in tracks.entries() {
            let is_video = matches!(entry.track_type(), Some(TrackType::Video));
            metadata.push((entry.track_number().unwrap_or(0), is_video));
        }
        if metadata.iter().any(|(number, _)| *number == 0) {
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "TrackNumber",
                reason: "a TrackEntry must state a non-zero TrackNumber",
            }));
        }
        let has_video = metadata.iter().any(|(_, is_video)| *is_video);
        let pending = metadata.iter().map(|_| None).collect();

        let head = vec![
            Write::Append(ebml),
            Write::Append(segment),
            Write::Append(seek),
            Write::Append(info_bytes),
            Write::Append(tracks_bytes),
        ];
        let scale = info.timestamp_scale().max(1);
        let ticks = layout.cluster_span.as_nanos() / u128::from(scale);
        Ok(Self {
            layout,
            span_ticks: i64::try_from(ticks).unwrap_or(i64::MAX),
            has_video,
            tracks: metadata,
            pending,
            cluster: Vec::new(),
            cluster_start: 0,
            cues: Vec::new(),
            payload_written: pos_tracks
                .checked_add(u64::try_from(tracks_bytes_len).map_err(|_| Error::SizeOverflow)?)
                .ok_or(Error::SizeOverflow)?,
            duration_ticks: 0,
            info_duration_offset: info_offset
                .checked_add(duration_in_info)
                .ok_or(Error::SizeOverflow)?,
            head,
            started: false,
        })
    }

    /// Queue one Block and return every write it completes.
    ///
    /// `timestamp` and `duration` are in Matroska Ticks under the `Info`'s
    /// `TimestampScale`. `duration` feeds `Info.Duration`; `None` states only
    /// this Block's instant. The first call also returns the header writes,
    /// because a stream is not a file until it has its first Block.
    pub fn push_block(
        &mut self,
        track: u64,
        timestamp: i64,
        duration: Option<u64>,
        flags: BlockFlags,
        frame: &[u8],
    ) -> Result<Vec<Write>> {
        let Some(index) = self.tracks.iter().position(|(number, _)| *number == track) else {
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "TrackNumber",
                reason: "Block names a track the Tracks element does not carry",
            }));
        };
        if let Some(duration) = duration {
            let end = timestamp
                .checked_add(i64::try_from(duration).map_err(|_| Error::SizeOverflow)?)
                .ok_or(Error::SizeOverflow)?;
            if end > self.duration_ticks {
                self.duration_ticks = end;
            }
        }
        let (number, is_video) = self.tracks[index];
        let mut writes = self.take_head();
        if let Some(previous) = self.pending[index].take()
            && let Some(bytes) = self.place(previous)?
        {
            writes.push(Write::Append(bytes));
        }
        self.pending[index] = Some(Pending {
            track_index: index,
            track_number: number,
            pts: timestamp,
            keyframe: flags.keyframe,
            discardable: flags.discardable,
            invisible: flags.invisible,
            is_video,
            data: frame.to_vec(),
        });
        while !self.pending.iter().any(Option::is_none) {
            let Some(index) = self.earliest() else {
                break;
            };
            if let Some(block) = self.pending[index].take()
                && let Some(bytes) = self.place(block)?
            {
                writes.push(Write::Append(bytes));
            }
        }
        Ok(writes)
    }

    /// End the open Cluster at this point: every held Block is placed first,
    /// then the Cluster is emitted, so the next [`push_block`](Self::push_block)
    /// opens a new one. A caller uses this for a caller-known boundary (a
    /// container-level timeline discontinuity) that the writer's own span and
    /// keyframe rules cannot see.
    pub fn cut_cluster(&mut self) -> Result<Vec<Write>> {
        let mut writes = Vec::new();
        while let Some(index) = self.earliest() {
            let Some(block) = self.pending[index].take() else {
                break;
            };
            if let Some(bytes) = self.place(block)? {
                writes.push(Write::Append(bytes));
            }
        }
        if !self.cluster.is_empty() {
            let bytes = self.emit_cluster()?;
            writes.push(Write::Append(bytes));
        }
        Ok(writes)
    }

    /// Flush every pending Block, the open Cluster, `Cues` and the
    /// `Info.Duration` patch.
    pub fn finish(&mut self) -> Result<Vec<Write>> {
        let mut writes = self.take_head();
        let mut held: Vec<Pending> = self.pending.iter_mut().filter_map(Option::take).collect();
        held.sort_by(|a, b| a.pts.cmp(&b.pts).then(a.track_index.cmp(&b.track_index)));
        for block in held {
            if let Some(bytes) = self.place(block)? {
                writes.push(Write::Append(bytes));
            }
        }
        if !self.cluster.is_empty() {
            let bytes = self.emit_cluster()?;
            writes.push(Write::Append(bytes));
        }
        let cues = self.build_cues()?;
        if !cues.is_empty() {
            writes.push(Write::Append(cues));
        }
        let duration = if self.duration_ticks > 0 {
            self.duration_ticks as f64
        } else {
            self.layout.duration_ticks.unwrap_or(0.0)
        };
        writes.push(Write::At {
            offset: self.info_duration_offset,
            bytes: duration.to_be_bytes().to_vec(),
        });
        Ok(writes)
    }

    fn take_head(&mut self) -> Vec<Write> {
        if self.started {
            return Vec::new();
        }
        self.started = true;
        core::mem::take(&mut self.head)
    }

    fn earliest(&self) -> Option<usize> {
        let mut best: Option<(i64, usize)> = None;
        for (index, pending) in self.pending.iter().enumerate() {
            let Some(block) = pending else {
                continue;
            };
            let key = (block.pts, index);
            if best.is_none_or(|best| key < best) {
                best = Some(key);
            }
        }
        best.map(|(_, index)| index)
    }

    fn place(&mut self, block: Pending) -> Result<Option<Vec<u8>>> {
        let new_cluster = starts_cluster(
            self.cluster.is_empty(),
            self.has_video,
            self.cluster_start,
            self.span_ticks,
            &block,
        );
        let mut finished = None;
        if new_cluster && !self.cluster.is_empty() {
            finished = Some(self.emit_cluster()?);
        }
        if self.cluster.is_empty() {
            self.cluster_start = block.pts.max(0);
        }
        self.cluster.push(block);
        Ok(finished)
    }

    fn emit_cluster(&mut self) -> Result<Vec<u8>> {
        let base = self.cluster.first().map_or(0, |block| block.pts.max(0));
        let mut rels = Vec::with_capacity(self.cluster.len());
        let mut body_len = timestamp_len(base)?;
        for block in &self.cluster {
            let relative = block.pts - base;
            let rel = i16::try_from(relative).map_err(|_| {
                Error::InvalidField(broadcast_core::error::InvalidField {
                    field: "SimpleBlock relative timestamp",
                    reason: "exceeds the signed 16-bit range",
                })
            })?;
            rels.push(rel);
            body_len = body_len
                .checked_add(simple_block_len(block.track_number, block.data.len())?)
                .ok_or(Error::SizeOverflow)?;
        }
        let mut out = Vec::with_capacity(body_len);
        append_master_start(&mut out, ids::CLUSTER, body_len)?;
        append_unsigned(&mut out, ids::TIMESTAMP, u64::try_from(base).unwrap_or(0))?;
        for (block, rel) in self.cluster.iter().zip(rels) {
            append_simple_block(&mut out, block.track_number, rel, block)?;
        }
        let offset = self.payload_written;
        for block in &self.cluster {
            if block.is_video && block.keyframe {
                self.cues.push(Cue {
                    time: block.pts.max(0),
                    track: block.track_number,
                    offset,
                });
            }
        }
        self.payload_written = self
            .payload_written
            .saturating_add(u64::try_from(out.len()).unwrap_or(u64::MAX));
        self.cluster.clear();
        Ok(out)
    }

    fn build_cues(&self) -> Result<Vec<u8>> {
        if self.cues.is_empty() {
            return Ok(Vec::new());
        }
        let mut points = Vec::with_capacity(self.cues.len());
        let mut last: Option<i64> = None;
        for cue in &self.cues {
            let raw = cue.time.max(0);
            let time = last.map_or(raw, |last| raw.max(last));
            last = Some(time);
            let mut positions = Vec::new();
            append_unsigned(&mut positions, ids::CUE_TRACK, cue.track)?;
            append_unsigned(&mut positions, ids::CUE_CLUSTER_POSITION, cue.offset)?;
            let mut wrapped_positions = Vec::new();
            append_master(&mut wrapped_positions, ids::CUE_TRACK_POSITIONS, &positions)?;
            let mut point_body = Vec::new();
            append_unsigned(
                &mut point_body,
                ids::CUE_TIME,
                u64::try_from(time).unwrap_or(0),
            )?;
            point_body.extend_from_slice(&wrapped_positions);
            append_master(&mut points, ids::CUE_POINT, &point_body)?;
        }
        let mut out = Vec::new();
        append_master(&mut out, ids::CUES, &points)?;
        Ok(out)
    }
}

/// Whether a Block starts a new Cluster: the first one, any video keyframe
/// when the stream has video, the caller's span, or a relative timestamp that
/// would leave the signed 16-bit range (RFC 9559 §12).
fn starts_cluster(
    empty: bool,
    has_video: bool,
    cluster_start: i64,
    max_span: i64,
    block: &Pending,
) -> bool {
    let relative = block.pts - cluster_start;
    let range = i64::from(i16::MIN)..=i64::from(i16::MAX);
    empty
        || (has_video && block.is_video && block.keyframe)
        || relative >= max_span
        || !range.contains(&relative)
}

fn simple_block_len(track: u64, data_len: usize) -> Result<usize> {
    let track_width = VintSize::minimal(track)?.width().get();
    let body = track_width
        .checked_add(TIMESTAMP_OCTETS)
        .and_then(|len| len.checked_add(FLAG_OCTETS))
        .and_then(|len| len.checked_add(data_len))
        .ok_or(Error::SizeOverflow)?;
    let element = ElementId::try_from_raw(ids::SIMPLE_BLOCK)?.octets();
    element
        .checked_add(
            VintSize::minimal(u64::try_from(body).map_err(|_| Error::SizeOverflow)?)?
                .width()
                .get(),
        )
        .and_then(|len| len.checked_add(body))
        .ok_or(Error::SizeOverflow)
}

fn timestamp_len(base: i64) -> Result<usize> {
    let body = Unsigned::new(u64::try_from(base).unwrap_or(0)).serialized_len();
    Ok(ElementId::try_from_raw(ids::TIMESTAMP)?.octets()
        + VintSize::minimal(u64::try_from(body).map_err(|_| Error::SizeOverflow)?)?
            .width()
            .get()
        + body)
}

fn append_simple_block(
    out: &mut Vec<u8>,
    track: u64,
    relative: i16,
    block: &Pending,
) -> Result<()> {
    let track_size = VintSize::minimal(track)?;
    let mut flags = 0u8;
    if block.keyframe {
        flags |= FLAG_KEYFRAME;
    }
    if block.invisible {
        flags |= FLAG_INVISIBLE;
    }
    if block.discardable {
        flags |= FLAG_DISCARDABLE;
    }
    let body = track_size
        .width()
        .get()
        .checked_add(TIMESTAMP_OCTETS)
        .and_then(|len| len.checked_add(FLAG_OCTETS))
        .and_then(|len| len.checked_add(block.data.len()))
        .ok_or(Error::SizeOverflow)?;
    let size = VintSize::minimal(u64::try_from(body).map_err(|_| Error::SizeOverflow)?)?;
    let id = ElementId::try_from_raw(ids::SIMPLE_BLOCK)?;
    let start = out.len();
    out.resize(start + id.octets() + size.serialized_len() + body, 0);
    let mut used = id.write(&mut out[start..])?;
    used += size.write(&mut out[start + used..])?;
    used += track_size.write(&mut out[start + used..])?;
    let target = out
        .get_mut(start + used..start + used + TIMESTAMP_OCTETS + FLAG_OCTETS)
        .ok_or(Error::SizeOverflow)?;
    target[..TIMESTAMP_OCTETS].copy_from_slice(&relative.to_be_bytes());
    target[TIMESTAMP_OCTETS] = flags;
    used += TIMESTAMP_OCTETS + FLAG_OCTETS;
    let data_start = start + used;
    let data_end = data_start + block.data.len();
    let slot = out
        .get_mut(data_start..data_end)
        .ok_or(Error::SizeOverflow)?;
    slot.copy_from_slice(&block.data);
    Ok(())
}

fn duration_payload_offset(info: &Info<'_>) -> Result<u64> {
    let payload: usize = info.children().iter().map(Serialize::serialized_len).sum();
    let header = info
        .header()
        .resized(u64::try_from(payload).map_err(|_| Error::SizeOverflow)?)?;
    let mut position = header.serialized_len();
    for child in info.children() {
        let child_payload = child.header().payload_len().unwrap_or(0);
        let child_header = child.header().resized(child_payload)?;
        if child.id() == ids::DURATION {
            if child_payload != u64::try_from(size_of::<f64>()).unwrap_or(0) {
                return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                    field: "Duration",
                    reason: "must be an eight-octet float for the writer to patch it",
                }));
            }
            return u64::try_from(position + child_header.serialized_len())
                .map_err(|_| Error::SizeOverflow);
        }
        position = position.saturating_add(child.serialized_len());
    }
    Err(Error::InvalidField(broadcast_core::error::InvalidField {
        field: "Duration",
        reason: "Info carries no Duration element",
    }))
}

fn ebml_header() -> Result<Vec<u8>> {
    let mut body = Vec::new();
    append_unsigned(
        &mut body,
        crate::ebml::document::EBML_VERSION_ID,
        EBML_VERSION,
    )?;
    append_unsigned(
        &mut body,
        crate::ebml::document::EBML_READ_VERSION_ID,
        EBML_READ_VERSION,
    )?;
    append_unsigned(
        &mut body,
        crate::ebml::document::EBML_MAX_ID_LENGTH_ID,
        EBML_MAX_ID_LENGTH,
    )?;
    append_unsigned(
        &mut body,
        crate::ebml::document::EBML_MAX_SIZE_LENGTH_ID,
        EBML_MAX_SIZE_LENGTH,
    )?;
    append_text(&mut body, crate::ebml::document::DOC_TYPE_ID, DOC_TYPE)?;
    append_unsigned(
        &mut body,
        crate::ebml::document::DOC_TYPE_VERSION_ID,
        DOC_TYPE_VERSION,
    )?;
    append_unsigned(
        &mut body,
        crate::ebml::document::DOC_TYPE_READ_VERSION_ID,
        DOC_TYPE_READ_VERSION,
    )?;
    let mut out = Vec::new();
    append_master(&mut out, crate::ebml::document::EBML_ID, &body)?;
    Ok(out)
}

fn seek_head(pos_info: u64, pos_tracks: u64) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&seek_entry(ids::INFO, pos_info)?);
    body.extend_from_slice(&seek_entry(ids::TRACKS, pos_tracks)?);
    let mut out = Vec::new();
    append_master(&mut out, ids::SEEK_HEAD, &body)?;
    Ok(out)
}

fn seek_entry(target: u32, position: u64) -> Result<Vec<u8>> {
    let id = ElementId::try_from_raw(target)?;
    let mut id_bytes = vec![0u8; id.octets()];
    id.write(&mut id_bytes)?;
    let mut body = Vec::new();
    append_binary(&mut body, ids::SEEK_ID, &id_bytes)?;
    let width = IntegerWidth::try_new(SEEK_POSITION_WIDTH)?;
    let padded = Unsigned::with_width(position, width)?;
    append_scalar(&mut body, ids::SEEK_POSITION, &padded.try_to_bytes()?)?;
    let mut out = Vec::new();
    append_master(&mut out, ids::SEEK, &body)?;
    Ok(out)
}

fn append_master(out: &mut Vec<u8>, id: u32, body: &[u8]) -> Result<()> {
    let mut element = Vec::new();
    append_element(&mut element, id, Some(body))?;
    out.extend_from_slice(&element);
    Ok(())
}

fn append_master_start(out: &mut Vec<u8>, id: u32, body_len: usize) -> Result<()> {
    let id = ElementId::try_from_raw(id)?;
    let size = VintSize::minimal(u64::try_from(body_len).map_err(|_| Error::SizeOverflow)?)?;
    let start = out.len();
    out.resize(start + id.octets() + size.serialized_len(), 0);
    let used = id.write(&mut out[start..])?;
    size.write(&mut out[start + used..])?;
    Ok(())
}

fn append_element(out: &mut Vec<u8>, id: u32, body: Option<&[u8]>) -> Result<()> {
    let id = ElementId::try_from_raw(id)?;
    let known = body.map_or(UNKNOWN_SIZE, |bytes| {
        VintSize::minimal(u64::try_from(bytes.len()).unwrap_or(u64::MAX)).unwrap_or(UNKNOWN_SIZE)
    });
    let start = out.len();
    let body_len = body.map_or(0, <[u8]>::len);
    out.resize(start + id.octets() + known.serialized_len() + body_len, 0);
    let used = id.write(&mut out[start..])?;
    known.write(&mut out[start + used..])?;
    if let Some(body) = body {
        let payload = out
            .get_mut(start + used + known.serialized_len()..)
            .ok_or(Error::SizeOverflow)?;
        payload.copy_from_slice(body);
    }
    Ok(())
}

fn append_scalar(out: &mut Vec<u8>, id: u32, body: &[u8]) -> Result<()> {
    append_element(out, id, Some(body))
}

fn append_unsigned(out: &mut Vec<u8>, id: u32, value: u64) -> Result<()> {
    append_scalar(out, id, &Unsigned::new(value).try_to_bytes()?)
}

fn append_text(out: &mut Vec<u8>, id: u32, value: &str) -> Result<()> {
    append_scalar(out, id, value.as_bytes())
}

fn append_binary(out: &mut Vec<u8>, id: u32, value: &[u8]) -> Result<()> {
    append_scalar(out, id, value)
}
