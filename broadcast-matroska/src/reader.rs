//! The sans-IO Matroska/WebM reader: it asks the caller for structure bytes
//! and yields the Segment's metadata, then one Block frame at a time.
//!
//! The reader owns at most one Cluster header and one Block header. Frame data
//! never passes through it: a [`Step::Block`] says where a frame's bytes are,
//! and the caller fetches that range itself. A file is walked by element id and
//! declared size; an unknown-size `Segment` or `Cluster` ends where RFC 8794
//! §6.2 says it does, at the next element the schema makes a sibling
//! (`ids::SEGMENT_CHILD_IDS`), never by guessing a size.
//!
//! Driving is [`Reader::step`] / [`Reader::supply`]: each `step` returns either
//! a [`Step::Need`] range to supply or a typed step, and `supply` accepts
//! exactly the outstanding range. A seekable source may jump over the bytes
//! between two demands; a forward-only source has already passed them, so the
//! driver discards them through a fixed buffer.
//!
//! # Spec citations
//!
//! - EBML element framing and the unknown size: RFC 8794 §4-§6, §6.2
//!   (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`).
//! - Matroska Segment, Info, Tracks, Cluster, Block and Cues: RFC 9559 §5, §6,
//!   §11 (timestamps), §12 (Block structure), §20 (`Cues`), transcribed at
//!   `private/specs-verified/ietf/specs/rfc9559_matroska.txt` and
//!   `private/specs-verified/mpeg/transmux/webm/ebml-matroska.md`.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ebml::document::EbmlHeader;
use crate::ebml::vint::{MAX_ELEMENT_ID_OCTETS, MAX_VINT_OCTETS};
use broadcast_core::Parse;

use crate::ebml::ElementHeader;
use crate::error::{Error, Result};
use crate::limits::Limits;
use crate::matroska::accessors::DEFAULT_TIMESTAMP_SCALE;
use crate::matroska::block::{BlockHeader, LaceHeader};
use crate::matroska::ids;
use crate::matroska::types::TrackType;
use crate::matroska::{Element, Info, SeekHead, Tracks};

/// Largest bytes an element header occupies: a four-octet Element ID and an
/// eight-octet Element Data Size (RFC 8794 §5, §6.1).
pub const MAX_ELEMENT_HEADER_BYTES: u64 = (MAX_ELEMENT_ID_OCTETS + MAX_VINT_OCTETS) as u64;

/// Octets an Element ID occupies on the wire, for the permissive resync scan.
const RESYNC_ID_BYTES: usize = MAX_ELEMENT_ID_OCTETS;

/// Bytes a resync scan keeps from the previous chunk so an id straddling two
/// chunks is still found.
const RESYNC_OVERLAP: usize = RESYNC_ID_BYTES - 1;

/// A half-open byte range in the input: `[offset, offset + len)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    /// First byte of the range.
    pub offset: u64,
    /// Number of bytes in the range.
    pub len: u64,
}

impl ByteRange {
    /// One past the last byte of the range.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.len)
    }
}

/// Whether the caller's byte source can seek backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// Every requested range can be served, including a backward seek. `len`
    /// is the input's total length.
    Seekable {
        /// Total input length in bytes.
        len: u64,
    },
    /// Ranges must not move backwards.
    ForwardOnly,
}

impl Source {
    /// The source's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Seekable { .. } => "seekable",
            Self::ForwardOnly => "forward-only",
        }
    }

    /// The total input length, when the source knows it.
    #[must_use]
    pub const fn total_len(&self) -> Option<u64> {
        match self {
            Self::Seekable { len } => Some(*len),
            Self::ForwardOnly => None,
        }
    }
}

broadcast_core::impl_spec_display!(Source);

/// How strictly the reader treats known real-world faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// Any violation is an error.
    #[default]
    Strict,
    /// A malformed element is reported as a [`Warning`] and the walk resumes at
    /// the next level-1 element id.
    Permissive,
}

impl Mode {
    /// The mode's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Permissive => "permissive",
        }
    }
}

broadcast_core::impl_spec_display!(Mode);

/// How a [`Reader::seek_to`] found its Cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SeekMethod {
    /// A `CuePoint` at or before the requested time named the Cluster.
    Cues,
    /// The Cluster index recorded by the walk named the Cluster.
    ClusterScan,
}

impl SeekMethod {
    /// The method's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Cues => "cues",
            Self::ClusterScan => "cluster-scan",
        }
    }
}

broadcast_core::impl_spec_display!(SeekMethod);

/// Where a seek positioned the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekOutcome {
    /// Absolute offset of the Cluster header the walk now reads from.
    pub offset: u64,
    /// The cue point's or Cluster header's Matroska Tick timestamp.
    pub timestamp: i64,
    /// How the Cluster was found.
    pub method: SeekMethod,
}

/// The Segment's timing facts (`Info`, RFC 9559 §5.1.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentInfo {
    /// `TimestampScale` in nanoseconds per Matroska Tick (RFC 9559 §11).
    pub timestamp_scale_ns: u64,
    /// `Duration` in Matroska Ticks, when the file states one.
    pub duration: Option<f64>,
}

impl SegmentInfo {
    /// The `Info` element's byte range, header included.
    #[must_use]
    pub const fn timestamp_scale_ns(&self) -> u64 {
        self.timestamp_scale_ns
    }

    /// The `Duration` in Matroska Ticks.
    #[must_use]
    pub const fn duration(&self) -> Option<f64> {
        self.duration
    }
}

/// One track's facts, copied out of its `TrackEntry` (RFC 9559 §5.1.4.1).
///
/// The values are owned because a `TrackEntry` borrows its `CodecID` and
/// `CodecPrivate` from the file, while this table outlives the bytes the reader
/// asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    track_number: u64,
    track_uid: u64,
    track_type: Option<TrackType>,
    track_type_value: u64,
    codec_id: String,
    codec_private: Vec<u8>,
    default_duration_ns: Option<u64>,
    codec_delay_ns: Option<u64>,
    seek_pre_roll_ns: Option<u64>,
    pixel_width: Option<u64>,
    pixel_height: Option<u64>,
    channels: Option<u64>,
    sampling_frequency: Option<f64>,
}

impl TrackInfo {
    /// The 1-based `TrackNumber` that Blocks name.
    #[must_use]
    pub const fn track_number(&self) -> u64 {
        self.track_number
    }

    /// The `TrackUID`.
    #[must_use]
    pub const fn track_uid(&self) -> u64 {
        self.track_uid
    }

    /// The decoded `TrackType`, or `None` for a code this crate does not name.
    #[must_use]
    pub const fn track_type(&self) -> Option<TrackType> {
        self.track_type
    }

    /// The raw `TrackType` value, preserved even when it is unknown.
    #[must_use]
    pub const fn track_type_value(&self) -> u64 {
        self.track_type_value
    }

    /// The `CodecID` (`V_VP9`, `A_OPUS`, …).
    #[must_use]
    pub fn codec_id(&self) -> &str {
        &self.codec_id
    }

    /// The `CodecPrivate` bytes.
    #[must_use]
    pub fn codec_private(&self) -> &[u8] {
        &self.codec_private
    }

    /// `DefaultDuration` in nanoseconds, the per-frame cadence of a laced
    /// Block (RFC 9559 §11).
    #[must_use]
    pub const fn default_duration_ns(&self) -> Option<u64> {
        self.default_duration_ns
    }

    /// `CodecDelay` in nanoseconds (RFC 9559 §5.1.4.1.6): how much of the
    /// track's first decoded samples is decoder fill rather than programme.
    /// Opus states its pre-skip here, and in `CodecPrivate`'s `OpusHead` as
    /// well.
    #[must_use]
    pub const fn codec_delay_ns(&self) -> Option<u64> {
        self.codec_delay_ns
    }

    /// `SeekPreRoll` in nanoseconds (RFC 9559 §5.1.4.1.26): the samples a
    /// decoder must consume after a seek before its output is correct.
    #[must_use]
    pub const fn seek_pre_roll_ns(&self) -> Option<u64> {
        self.seek_pre_roll_ns
    }

    /// `PixelWidth`, when the track is video.
    #[must_use]
    pub const fn pixel_width(&self) -> Option<u64> {
        self.pixel_width
    }

    /// `PixelHeight`, when the track is video.
    #[must_use]
    pub const fn pixel_height(&self) -> Option<u64> {
        self.pixel_height
    }

    /// `Channels`, when the track is audio.
    #[must_use]
    pub const fn channels(&self) -> Option<u64> {
        self.channels
    }

    /// `SamplingFrequency`, when the track is audio.
    #[must_use]
    pub const fn sampling_frequency(&self) -> Option<f64> {
        self.sampling_frequency
    }
}

/// One `CuePoint`/`CueTrackPositions` pair, in the units `Cues` states
/// (RFC 9559 §5.1.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CueEntry {
    /// `CueTime` in Matroska Ticks.
    pub time: i64,
    /// `CueTrack`.
    pub track: u64,
    /// `CueClusterPosition`, relative to the Segment payload start.
    pub cluster_position: u64,
    /// `CueDuration` in Matroska Ticks, when present.
    pub duration: Option<u64>,
}

/// One whole element a Step hands to the caller, unparsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementRef {
    /// The element's id and encoded size.
    pub header: ElementHeader,
    /// Its whole byte range, header included.
    pub range: ByteRange,
}

impl ElementRef {
    /// The element's id.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.header.id().value()
    }
}

/// One frame of a `SimpleBlock` or `Block` (RFC 9559 §12).
///
/// Every frame of one laced Block carries the Block's own timestamp: Matroska
/// times the Block, and the frames' cadence comes from the track's
/// `DefaultDuration` (RFC 9559 §11). `lace_index` says which frame of the lace
/// this is, and `frame` is where its bytes are in the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockStep {
    /// The Block's 1-based `TrackNumber`.
    pub track: u64,
    /// Absolute presentation time in Matroska Ticks.
    pub timestamp: i64,
    /// The same instant in nanoseconds.
    pub timestamp_ns: i128,
    /// `BlockDuration` in Matroska Ticks, when the BlockGroup states one.
    pub duration: Option<u64>,
    /// Whether this Block starts a random-access point.
    pub keyframe: bool,
    /// The `SimpleBlock` discardable flag.
    pub discardable: bool,
    /// The `SimpleBlock` invisible flag.
    pub invisible: bool,
    /// `BlockGroup` `DiscardPadding` in nanoseconds (RFC 9559 §5.1.3.5.7):
    /// positive means silence at the end of the frame, negative at its start.
    pub discard_padding: Option<i64>,
    /// This frame's byte range in the source.
    pub frame: ByteRange,
    /// Zero-based index of this frame in the Block's lace.
    pub lace_index: u32,
    /// How many frames the Block's lace carries.
    pub lace_count: u32,
}

/// One step of the reader state machine.
#[derive(Debug)]
#[non_exhaustive]
pub enum Step<'a> {
    /// Supply exactly this range with [`Reader::supply`].
    Need(ByteRange),
    /// The Segment's `Info` facts and the element's byte range.
    Info {
        /// `TimestampScale` and `Duration`.
        info: SegmentInfo,
        /// The `Info` element, header included.
        element: ByteRange,
    },
    /// The Segment's tracks, once, after `Info`.
    Tracks {
        /// The track table, in `TrackEntry` order.
        tracks: &'a [TrackInfo],
        /// The `Tracks` element, header included.
        element: ByteRange,
    },
    /// One frame of one Block.
    Block(BlockStep),
    /// The Segment's `Cues`, once.
    Cues(&'a [CueEntry]),
    /// A `Chapters` element; its bytes are at `ElementRef::range`.
    Chapters(ElementRef),
    /// A `Tags` element; its bytes are at `ElementRef::range`.
    Tags(ElementRef),
    /// An `Attachments` element; its bytes are at `ElementRef::range`.
    Attachments(ElementRef),
    /// A fault [`Mode::Permissive`] tolerated.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// A fault [`Mode::Permissive`] tolerated, reported once as [`Step::Warning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// An element header did not parse; the walk resumed at the next level-1 id.
    MalformedElement {
        /// Absolute offset of the malformed header.
        offset: u64,
    },
    /// A buffered master element did not parse; it was skipped.
    UnparsedElement {
        /// The element's id.
        id: u32,
        /// Absolute offset of the element.
        offset: u64,
    },
    /// An element used the unknown size where RFC 8794 §6.2 does not allow it;
    /// the walk resumed at the next level-1 id.
    UnknownSizeElement {
        /// The element's id.
        id: u32,
        /// Absolute offset of the element.
        offset: u64,
    },
    /// A Block did not parse; it was skipped.
    MalformedBlock {
        /// Absolute offset of the Block element.
        offset: u64,
    },
    /// The input ended inside an element; the steps read so far stand.
    Truncated {
        /// Absolute offset where the input ended.
        offset: u64,
    },
}

impl Warning {
    /// The warning's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::MalformedElement { .. } => "malformed-element",
            Self::UnparsedElement { .. } => "unparsed-element",
            Self::UnknownSizeElement { .. } => "unknown-size-element",
            Self::MalformedBlock { .. } => "malformed-block",
            Self::Truncated { .. } => "truncated",
        }
    }

    /// The absolute offset the fault starts at.
    #[must_use]
    pub const fn position(&self) -> u64 {
        match self {
            Self::MalformedElement { offset }
            | Self::UnparsedElement { offset, .. }
            | Self::UnknownSizeElement { offset, .. }
            | Self::MalformedBlock { offset }
            | Self::Truncated { offset } => *offset,
        }
    }
}

broadcast_core::impl_spec_display!(Warning);

/// Where the next element header lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Where {
    /// The document root: the EBML header and `Segment`s.
    Top,
    /// A `Segment`'s children.
    Segment,
    /// A `Cluster`'s children.
    Cluster,
    /// A `BlockGroup`'s children, ending at `end`.
    Group { end: u64 },
}

/// The element kind a buffered level-1 master is parsed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MasterKind {
    Ebml,
    SeekHead,
    Info,
    Tracks,
    Cues,
}

/// The context of a small unsigned leaf the reader buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsignedKind {
    ClusterTimestamp,
    BlockDuration,
}

/// The context of a small signed leaf the reader buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignedKind {
    DiscardPadding,
}

/// Whether a Block sits directly in a Cluster or inside a BlockGroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockOwner {
    Cluster,
    Group,
}

/// What the outstanding [`Step::Need`] buys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Header {
        at: Where,
    },
    Master {
        kind: MasterKind,
        offset: u64,
        header: ElementHeader,
        body: ByteRange,
    },
    Unsigned {
        kind: UnsignedKind,
        offset: u64,
        header: ElementHeader,
        body: ByteRange,
    },
    Signed {
        kind: SignedKind,
        offset: u64,
        header: ElementHeader,
        body: ByteRange,
    },
    Block {
        owner: BlockOwner,
        header: ElementHeader,
        element: ByteRange,
        payload: ByteRange,
    },
    Resync {
        remaining: u64,
        at: Where,
    },
    Done,
}

/// A `Cues` element read out of order through a `SeekHead` target.
#[derive(Debug, Clone, Copy)]
struct SideRead {
    return_pos: u64,
    header_seen: bool,
}

/// One Cluster header recorded for a cue-less seek (RFC 9559 §20).
#[derive(Debug, Clone, Copy)]
struct ClusterIndexEntry {
    offset: u64,
    timestamp: i64,
}

/// One Block's frame ranges, held while its `BlockGroup` is read.
#[derive(Debug, Clone)]
struct PendingFrames {
    track: u64,
    timestamp: i64,
    discardable: bool,
    invisible: bool,
    frames: Vec<ByteRange>,
}

/// A `BlockGroup` being read: its end and the facts that decide its one Block.
#[derive(Debug, Clone)]
struct GroupContext {
    end: u64,
    reference: bool,
    duration: Option<u64>,
    discard_padding: Option<i64>,
    block: Option<PendingFrames>,
}

/// The sans-IO Matroska reader. See the [module docs](self).
#[derive(Debug)]
pub struct Reader {
    source: Source,
    mode: Mode,
    limits: Limits,
    pos: u64,
    highest_requested: u64,
    pending: Option<ByteRange>,
    phase: Phase,
    done: bool,
    seen_ebml: bool,
    segment_started: bool,
    segment_payload_start: u64,
    segment_end: Option<u64>,
    timestamp_scale_ns: u64,
    info: Option<SegmentInfo>,
    info_element: Option<ByteRange>,
    tracks: Vec<TrackInfo>,
    tracks_element: Option<ByteRange>,
    tracks_parsed: bool,
    cues: Vec<CueEntry>,
    cues_parsed: bool,
    seek_entries: Vec<(u32, u64)>,
    emitted_info: bool,
    emitted_tracks: bool,
    emitted_cues: bool,
    warnings: VecDeque<Warning>,
    blocks: VecDeque<BlockStep>,
    elements: VecDeque<(u32, ElementRef)>,
    held: Option<(ElementHeader, u64)>,
    side: Option<SideRead>,
    cluster_index: Vec<ClusterIndexEntry>,
    current_cluster: Option<usize>,
    cluster_end: Option<u64>,
    cluster_timestamp: i64,
    group: Option<GroupContext>,
    resync_carry: [u8; RESYNC_OVERLAP],
    resync_carry_len: usize,
}

impl Reader {
    /// Create a reader. It issues its first [`Step::Need`] from the first
    /// [`Reader::step`].
    #[must_use]
    pub fn new(source: Source, mode: Mode, limits: Limits) -> Self {
        Self {
            source,
            mode,
            limits,
            pos: 0,
            highest_requested: 0,
            pending: None,
            phase: Phase::Header { at: Where::Top },
            done: false,
            seen_ebml: false,
            segment_started: false,
            segment_payload_start: 0,
            segment_end: None,
            timestamp_scale_ns: DEFAULT_TIMESTAMP_SCALE,
            info: None,
            info_element: None,
            tracks: Vec::new(),
            tracks_element: None,
            tracks_parsed: false,
            cues: Vec::new(),
            cues_parsed: false,
            seek_entries: Vec::new(),
            emitted_info: false,
            emitted_tracks: false,
            emitted_cues: false,
            warnings: VecDeque::new(),
            blocks: VecDeque::new(),
            elements: VecDeque::new(),
            held: None,
            side: None,
            cluster_index: Vec::new(),
            current_cluster: None,
            cluster_end: None,
            cluster_timestamp: 0,
            group: None,
            resync_carry: [0; RESYNC_OVERLAP],
            resync_carry_len: 0,
        }
    }

    /// The timestamp scale in nanoseconds per Matroska Tick, as the Segment
    /// states it so far (RFC 9559 §11).
    #[must_use]
    pub const fn timestamp_scale_ns(&self) -> u64 {
        self.timestamp_scale_ns
    }

    /// The tracks read so far.
    #[must_use]
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// The cue entries read so far.
    #[must_use]
    pub fn cues(&self) -> &[CueEntry] {
        &self.cues
    }

    /// Supply the outstanding [`Step::Need`] range.
    ///
    /// `offset` must equal the requested offset and `bytes` may not be longer
    /// than the requested range; a shorter supply means the input ended there.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Err(Error::SupplyWithoutNeed);
        };
        if offset != pending.offset {
            self.pending = Some(pending);
            return Err(Error::SupplyMismatch {
                expected: pending.offset,
                got: offset,
            });
        }
        let supplied = u64::try_from(bytes.len()).map_err(|_| Error::SizeOverflow)?;
        if supplied > pending.len {
            self.pending = Some(pending);
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "supply",
                reason: "supplied range is longer than the reader asked for",
            }));
        }
        self.accept(pending, bytes)
    }

    fn request(&mut self, range: ByteRange) -> Result<()> {
        if let Source::ForwardOnly = self.source
            && range.offset < self.highest_requested
        {
            return Err(Error::BackwardSeek {
                offset: range.offset,
                position: self.highest_requested,
            });
        }
        self.highest_requested = self.highest_requested.max(range.offset);
        self.pending = Some(range);
        Ok(())
    }

    fn source_len(&self) -> Option<u64> {
        self.source.total_len()
    }

    fn end_for(&self, at: Where) -> Option<u64> {
        match at {
            Where::Top => self.source_len(),
            Where::Segment => self.segment_end.or_else(|| self.source_len()),
            Where::Cluster => self
                .cluster_end
                .or(self.segment_end)
                .or_else(|| self.source_len()),
            Where::Group { end } => Some(end),
        }
    }

    fn short_supply(&self, range: ByteRange, bytes: &[u8]) -> bool {
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) < range.len
    }

    fn warned(&mut self, warning: Warning) {
        self.warnings.push_back(warning);
    }

    fn tolerated(&self) -> bool {
        self.mode == Mode::Permissive
    }
}

impl Reader {
    /// The next step.
    ///
    /// Named `step`, not `next`: each step borrows the reader (the
    /// [`Step::Tracks`] arm hands out its own track table), which
    /// [`Iterator::next`]'s signature cannot express.
    pub fn step(&mut self) -> Result<Step<'_>> {
        loop {
            if let Some(warning) = self.warnings.pop_front() {
                return Ok(Step::Warning(warning));
            }
            if let Some(pending) = self.pending {
                return Ok(Step::Need(pending));
            }
            if !self.emitted_info
                && let (Some(info), Some(element)) = (self.info, self.info_element)
            {
                self.emitted_info = true;
                return Ok(Step::Info { info, element });
            }
            if !self.emitted_tracks && self.tracks_parsed {
                self.emitted_tracks = true;
                let element = self
                    .tracks_element
                    .unwrap_or(ByteRange { offset: 0, len: 0 });
                return Ok(Step::Tracks {
                    tracks: &self.tracks,
                    element,
                });
            }
            if !self.emitted_cues && self.cues_parsed && (self.tracks_parsed || self.done) {
                self.emitted_cues = true;
                return Ok(Step::Cues(&self.cues));
            }
            if let Some(block) = self.blocks.pop_front() {
                return Ok(Step::Block(block));
            }
            if let Some((id, element)) = self.elements.pop_front() {
                return Ok(match id {
                    ids::CHAPTERS => Step::Chapters(element),
                    ids::TAGS => Step::Tags(element),
                    _ => Step::Attachments(element),
                });
            }
            if self.done {
                return Ok(Step::Done);
            }
            match self.phase {
                Phase::Done => {
                    self.done = true;
                }
                Phase::Header { at } => self.drive_header(at)?,
                Phase::Resync { remaining, at } => self.drive_resync(remaining, at)?,
                Phase::Master { body, .. }
                | Phase::Unsigned { body, .. }
                | Phase::Signed { body, .. } => {
                    self.request(body)?;
                }
                Phase::Block { payload, .. } => {
                    let prefix = self.prefix(payload);
                    self.request(prefix)?;
                }
            }
        }
    }

    fn drive_header(&mut self, at: Where) -> Result<()> {
        if let Some((header, offset)) = self.held.take() {
            return self.dispatch_header(at, header, offset);
        }
        let end = self.end_for(at);
        if let Some(end) = end
            && self.pos >= end
        {
            return self.close(at);
        }
        let len = match end {
            Some(end) => end.saturating_sub(self.pos).min(MAX_ELEMENT_HEADER_BYTES),
            None => MAX_ELEMENT_HEADER_BYTES,
        };
        if len == 0 {
            return self.close(at);
        }
        self.request(ByteRange {
            offset: self.pos,
            len,
        })
    }

    fn drive_resync(&mut self, remaining: u64, at: Where) -> Result<()> {
        let end = self.end_for(at);
        if let Some(end) = end
            && self.pos >= end
        {
            return self.end_resync(at);
        }
        let hard = remaining.min(self.limits.max_read_bytes());
        let len = match end {
            Some(end) => hard.min(end.saturating_sub(self.pos)),
            None => hard,
        };
        if len == 0 {
            return self.end_resync(at);
        }
        self.request(ByteRange {
            offset: self.pos,
            len,
        })
    }

    fn end_resync(&mut self, at: Where) -> Result<()> {
        self.resync_carry_len = 0;
        if self.tolerated() {
            self.done = true;
            self.phase = Phase::Done;
            return Ok(());
        }
        self.phase = Phase::Header { at };
        Err(Error::InvalidField(broadcast_core::error::InvalidField {
            field: "resync",
            reason: "no level-1 element id within the resync cap",
        }))
    }

    fn close(&mut self, at: Where) -> Result<()> {
        match at {
            Where::Top => {
                self.done = true;
                self.phase = Phase::Done;
            }
            Where::Segment => {
                if self.segment_end.take().is_some() {
                    self.phase = Phase::Header { at: Where::Top };
                } else {
                    self.done = true;
                    self.phase = Phase::Done;
                }
            }
            Where::Cluster => {
                self.finish_cluster();
                self.phase = Phase::Header { at: Where::Segment };
            }
            Where::Group { .. } => {
                self.finish_group()?;
                self.phase = Phase::Header { at: Where::Cluster };
            }
        }
        Ok(())
    }

    fn finish_cluster(&mut self) {
        self.cluster_end = None;
        self.group = None;
        self.current_cluster = None;
        self.cluster_timestamp = 0;
    }

    fn finish_group(&mut self) -> Result<()> {
        if let Some(group) = self.group.take()
            && let Some(block) = group.block
        {
            self.emit_frames(
                block,
                !group.reference,
                group.duration,
                group.discard_padding,
            )?;
        }
        Ok(())
    }

    fn malformed(&mut self, at: Where, offset: u64, error: Error) -> Result<()> {
        if !self.tolerated() {
            return Err(error);
        }
        self.warned(Warning::MalformedElement { offset });
        match at {
            Where::Cluster => {
                self.finish_cluster();
                self.phase = Phase::Header { at: Where::Segment };
            }
            Where::Group { .. } => {
                self.finish_group()?;
                self.phase = Phase::Header { at: Where::Cluster };
            }
            Where::Top | Where::Segment => {}
        }
        self.pos = offset.saturating_add(1);
        self.resync_carry_len = 0;
        let resync_at = match at {
            Where::Group { .. } => Where::Cluster,
            other => other,
        };
        self.phase = Phase::Resync {
            remaining: self.limits.max_resync_bytes(),
            at: resync_at,
        };
        Ok(())
    }

    fn truncated(&mut self, at: Where, offset: u64, error: Error) -> Result<()> {
        if !self.tolerated() {
            return Err(error);
        }
        self.warned(Warning::Truncated { offset });
        self.done = true;
        self.phase = Phase::Done;
        let _ = at;
        Ok(())
    }
}

impl Reader {
    fn accept(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        match self.phase {
            Phase::Header { at } => self.accept_header(at, range, bytes),
            Phase::Master {
                kind,
                offset,
                header,
                body,
            } => self.accept_master(kind, offset, header, body, range, bytes),
            Phase::Unsigned {
                kind, offset, body, ..
            } => self.accept_unsigned(kind, offset, body, range, bytes),
            Phase::Signed {
                kind, offset, body, ..
            } => self.accept_signed(kind, offset, body, range, bytes),
            Phase::Block {
                owner,
                header,
                element,
                payload,
            } => self.accept_block(owner, header, element, payload, range, bytes),
            Phase::Resync { remaining, at } => self.accept_resync(remaining, at, range, bytes),
            Phase::Done => Ok(()),
        }
    }

    fn accept_header(&mut self, at: Where, range: ByteRange, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return self.end_of_input(at, range.offset);
        }
        let (header, used) = match ElementHeader::parse(bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                if self.short_supply(range, bytes) {
                    return self.truncated(at, range.offset, error);
                }
                return self.malformed(at, range.offset, error);
            }
        };
        self.pos = range
            .offset
            .saturating_add(u64::try_from(used).unwrap_or(u64::MAX));
        self.dispatch_header(at, header, range.offset)
    }

    fn end_of_input(&mut self, at: Where, offset: u64) -> Result<()> {
        match at {
            Where::Group { end } if self.pos < end => self.truncated(
                at,
                offset,
                Error::Truncated {
                    what: "BlockGroup",
                    need: usize::try_from(end.saturating_sub(self.pos)).unwrap_or(usize::MAX),
                    have: 0,
                },
            ),
            _ => self.close(at),
        }
    }

    fn dispatch_header(&mut self, at: Where, header: ElementHeader, offset: u64) -> Result<()> {
        let id = header.id().value();
        let payload_len = header.payload_len();
        if payload_len.is_none() && id != ids::SEGMENT && id != ids::CLUSTER {
            if !self.tolerated() {
                return Err(Error::UnknownSizeNotAllowed { id });
            }
            self.warned(Warning::UnknownSizeElement { id, offset });
            self.pos = self.pos.max(offset.saturating_add(1));
            self.resync_carry_len = 0;
            self.phase = Phase::Resync {
                remaining: self.limits.max_resync_bytes(),
                at,
            };
            return Ok(());
        }
        if let Some(len) = payload_len
            && len > self.limits.max_element_bytes()
        {
            return Err(Error::LimitExceeded {
                limit: "max_element_bytes",
                value: len,
            });
        }
        match at {
            Where::Top => self.dispatch_top(header, offset, payload_len),
            Where::Segment => {
                if let Some(side) = self.side
                    && !side.header_seen
                {
                    if id != ids::CUES {
                        self.warned(Warning::MalformedElement { offset });
                        self.pos = side.return_pos;
                        self.side = None;
                        return Ok(());
                    }
                    if let Some(side) = self.side.as_mut() {
                        side.header_seen = true;
                    }
                }
                self.dispatch_segment_child(header, offset, payload_len)
            }
            Where::Cluster => self.dispatch_cluster_child(header, offset, payload_len),
            Where::Group { .. } => self.dispatch_group_child(header, offset, payload_len),
        }
    }

    fn dispatch_top(
        &mut self,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        if id == crate::ebml::document::EBML_ID {
            return self.start_master(MasterKind::Ebml, header, offset, payload_len);
        }
        if id == ids::SEGMENT {
            self.seen_ebml = true;
            return self.handle_segment(header);
        }
        if !self.tolerated() {
            return Err(Error::UnexpectedElement {
                expected: ids::SEGMENT,
                found: id,
            });
        }
        self.warned(Warning::UnparsedElement { id, offset });
        self.skip(payload_len, offset, Where::Top)
    }

    fn handle_segment(&mut self, header: ElementHeader) -> Result<()> {
        let payload_start = self.pos;
        self.segment_started = true;
        self.segment_payload_start = payload_start;
        self.segment_end = match header.payload_len() {
            Some(len) => Some(payload_start.checked_add(len).ok_or(Error::SizeOverflow)?),
            None => None,
        };
        self.timestamp_scale_ns = DEFAULT_TIMESTAMP_SCALE;
        self.info = None;
        self.info_element = None;
        self.tracks.clear();
        self.tracks_element = None;
        self.tracks_parsed = false;
        self.cues.clear();
        self.cues_parsed = false;
        self.seek_entries.clear();
        self.emitted_info = false;
        self.emitted_tracks = false;
        self.emitted_cues = false;
        self.side = None;
        self.cluster_index.clear();
        self.current_cluster = None;
        self.cluster_end = None;
        self.cluster_timestamp = 0;
        self.group = None;
        self.held = None;
        self.phase = Phase::Header { at: Where::Segment };
        Ok(())
    }

    fn dispatch_segment_child(
        &mut self,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        match id {
            ids::SEGMENT => {
                self.seen_ebml = true;
                self.handle_segment(header)
            }
            ids::SEEK_HEAD => self.start_master(MasterKind::SeekHead, header, offset, payload_len),
            ids::INFO => self.start_master(MasterKind::Info, header, offset, payload_len),
            ids::TRACKS => self.start_master(MasterKind::Tracks, header, offset, payload_len),
            ids::CUES => {
                if self.cues_parsed {
                    return self.skip(payload_len, offset, Where::Segment);
                }
                self.start_master(MasterKind::Cues, header, offset, payload_len)
            }
            ids::CHAPTERS | ids::TAGS | ids::ATTACHMENTS => {
                let Some(len) = payload_len else {
                    return Err(Error::UnknownSizeNotAllowed { id });
                };
                let end = self.pos.checked_add(len).ok_or(Error::SizeOverflow)?;
                self.elements.push_back((
                    id,
                    ElementRef {
                        header,
                        range: ByteRange {
                            offset,
                            len: end.saturating_sub(offset),
                        },
                    },
                ));
                self.pos = end;
                self.phase = Phase::Header { at: Where::Segment };
                Ok(())
            }
            ids::CLUSTER => self.enter_cluster(header, offset, payload_len),
            _ if id == crate::ebml::global::VOID_ID || id == crate::ebml::global::CRC32_ID => {
                self.skip(payload_len, offset, Where::Segment)
            }
            _ => self.malformed(
                Where::Segment,
                offset,
                Error::UnexpectedElement {
                    expected: ids::CLUSTER,
                    found: id,
                },
            ),
        }
    }

    fn enter_cluster(
        &mut self,
        _header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let payload_start = self.pos;
        self.cluster_end = match payload_len {
            Some(len) => Some(payload_start.checked_add(len).ok_or(Error::SizeOverflow)?),
            None => None,
        };
        self.cluster_timestamp = 0;
        self.group = None;
        let capacity = usize::try_from(self.limits.max_clusters()).unwrap_or(usize::MAX);
        if self.cluster_index.len() < capacity {
            self.cluster_index.push(ClusterIndexEntry {
                offset,
                timestamp: 0,
            });
            self.current_cluster = Some(self.cluster_index.len() - 1);
        } else {
            self.current_cluster = None;
        }
        self.phase = Phase::Header { at: Where::Cluster };
        Ok(())
    }

    fn dispatch_cluster_child(
        &mut self,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        if is_level_one(id) {
            self.finish_cluster();
            self.held = Some((header, offset));
            self.phase = Phase::Header { at: Where::Segment };
            return Ok(());
        }
        match id {
            ids::TIMESTAMP => {
                self.start_unsigned(UnsignedKind::ClusterTimestamp, header, offset, payload_len)
            }
            ids::SIMPLE_BLOCK => self.start_block(BlockOwner::Cluster, header, offset, payload_len),
            ids::BLOCK_GROUP => {
                let Some(len) = payload_len else {
                    return Err(Error::UnknownSizeNotAllowed { id });
                };
                let end = self.pos.checked_add(len).ok_or(Error::SizeOverflow)?;
                self.group = Some(GroupContext {
                    end,
                    reference: false,
                    duration: None,
                    discard_padding: None,
                    block: None,
                });
                self.phase = Phase::Header {
                    at: Where::Group { end },
                };
                Ok(())
            }
            _ if id == crate::ebml::global::VOID_ID || id == crate::ebml::global::CRC32_ID => {
                self.skip(payload_len, offset, Where::Cluster)
            }
            _ => self.malformed(
                Where::Cluster,
                offset,
                Error::UnexpectedElement {
                    expected: ids::CLUSTER,
                    found: id,
                },
            ),
        }
    }

    fn dispatch_group_child(
        &mut self,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        match id {
            ids::BLOCK => self.start_block(BlockOwner::Group, header, offset, payload_len),
            ids::REFERENCE_BLOCK => {
                if let Some(group) = self.group.as_mut() {
                    group.reference = true;
                }
                self.skip(payload_len, offset, self.group_where())
            }
            ids::BLOCK_DURATION => {
                self.start_unsigned(UnsignedKind::BlockDuration, header, offset, payload_len)
            }
            ids::DISCARD_PADDING => {
                self.start_signed(SignedKind::DiscardPadding, header, offset, payload_len)
            }
            _ => self.skip(payload_len, offset, self.group_where()),
        }
    }

    fn group_where(&self) -> Where {
        match self.group.as_ref() {
            Some(group) => Where::Group { end: group.end },
            None => Where::Cluster,
        }
    }

    fn skip(&mut self, payload_len: Option<u64>, offset: u64, at: Where) -> Result<()> {
        let Some(len) = payload_len else {
            return Err(Error::UnknownSizeNotAllowed { id: 0 });
        };
        let end = self.pos.checked_add(len).ok_or(Error::SizeOverflow)?;
        if let Where::Group { end: group_end } = at
            && end > group_end
        {
            return self.overrun(offset, end, group_end, at);
        }
        if let Where::Cluster = at
            && let Some(cluster_end) = self.cluster_end
            && end > cluster_end
        {
            return self.overrun(offset, end, cluster_end, at);
        }
        if let Where::Segment = at
            && let Some(segment_end) = self.segment_end
            && end > segment_end
        {
            return self.overrun(offset, end, segment_end, at);
        }
        self.pos = end;
        self.phase = Phase::Header { at };
        Ok(())
    }

    fn overrun(&mut self, offset: u64, _end: u64, _parent_end: u64, at: Where) -> Result<()> {
        if !self.tolerated() {
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "element size",
                reason: "element runs past its parent",
            }));
        }
        self.malformed(
            at,
            offset,
            Error::InvalidField(broadcast_core::error::InvalidField {
                field: "element size",
                reason: "element runs past its parent",
            }),
        )
    }

    fn start_master(
        &mut self,
        kind: MasterKind,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        let Some(len) = payload_len else {
            return Err(Error::UnknownSizeNotAllowed { id });
        };
        if len > self.limits.max_master_bytes() {
            return Err(Error::LimitExceeded {
                limit: "max_master_bytes",
                value: len,
            });
        }
        let body = ByteRange {
            offset: self.pos,
            len,
        };
        self.phase = Phase::Master {
            kind,
            offset,
            header,
            body,
        };
        self.request(body)
    }

    fn start_unsigned(
        &mut self,
        kind: UnsignedKind,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        let Some(len) = payload_len else {
            return Err(Error::UnknownSizeNotAllowed { id });
        };
        let body = ByteRange {
            offset: self.pos,
            len,
        };
        self.phase = Phase::Unsigned {
            kind,
            offset,
            header,
            body,
        };
        self.request(body)
    }

    fn start_signed(
        &mut self,
        kind: SignedKind,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        let Some(len) = payload_len else {
            return Err(Error::UnknownSizeNotAllowed { id });
        };
        let body = ByteRange {
            offset: self.pos,
            len,
        };
        self.phase = Phase::Signed {
            kind,
            offset,
            header,
            body,
        };
        self.request(body)
    }

    fn start_block(
        &mut self,
        owner: BlockOwner,
        header: ElementHeader,
        offset: u64,
        payload_len: Option<u64>,
    ) -> Result<()> {
        let id = header.id().value();
        let Some(len) = payload_len else {
            return Err(Error::UnknownSizeNotAllowed { id });
        };
        let payload = ByteRange {
            offset: self.pos,
            len,
        };
        let end = payload.end();
        if let BlockOwner::Group = owner {
            let group_end = self.group.as_ref().map_or(end, |group| group.end);
            if end > group_end {
                self.warned(Warning::MalformedBlock { offset });
                self.pos = group_end;
                self.phase = Phase::Header {
                    at: Where::Group { end: group_end },
                };
                return Ok(());
            }
        } else if let Some(cluster_end) = self.cluster_end
            && end > cluster_end
        {
            return self.overrun(offset, end, cluster_end, Where::Cluster);
        }
        self.phase = Phase::Block {
            owner,
            header,
            element: ByteRange {
                offset,
                len: end.saturating_sub(offset),
            },
            payload,
        };
        let prefix = self.prefix(payload);
        self.request(prefix)
    }

    fn prefix(&self, payload: ByteRange) -> ByteRange {
        ByteRange {
            offset: payload.offset,
            len: payload.len.min(self.limits.max_block_header_bytes()),
        }
    }
}

impl Reader {
    fn accept_master(
        &mut self,
        kind: MasterKind,
        offset: u64,
        header: ElementHeader,
        body: ByteRange,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<()> {
        if self.short_supply(range, bytes) {
            return self.truncated(
                Where::Segment,
                offset,
                Error::Truncated {
                    what: "master element",
                    need: usize::try_from(body.len).unwrap_or(usize::MAX),
                    have: bytes.len(),
                },
            );
        }
        let full = element_bytes(header, bytes)?;
        self.pos = body.end();
        self.phase = Phase::Header { at: Where::Segment };
        if let MasterKind::Tracks = kind
            && self.tolerated()
        {
            let tracks = self.parse_tracks_tolerantly(&full, header, offset)?;
            return self.accept_tracks(Some(&tracks), offset, body);
        }
        let parsed = Element::parse_at(&full, &self.limits);
        let element = match parsed {
            Ok((element, _)) => element,
            Err(error) => return self.master_failed(kind, offset, error),
        };
        match kind {
            MasterKind::Ebml => {
                EbmlHeader::parse(&full)?;
                self.seen_ebml = true;
                self.phase = Phase::Header { at: Where::Top };
                Ok(())
            }
            MasterKind::SeekHead => self.accept_seek_head(element.as_seek_head()),
            MasterKind::Info => match self.accept_info(element.as_info(), offset, body) {
                Ok(()) => Ok(()),
                Err(_error) if self.tolerated() => {
                    self.warned(Warning::MalformedElement { offset });
                    Ok(())
                }
                Err(error) => Err(error),
            },
            MasterKind::Tracks => self.accept_tracks(element.as_tracks(), offset, body),
            MasterKind::Cues => self.accept_cues(element.as_cues(), offset, body),
        }
    }

    /// Type each `TrackEntry` on its own in [`Mode::Permissive`]: a corrupt
    /// entry is reported and skipped, the others are announced.
    fn parse_tracks_tolerantly<'a>(
        &mut self,
        full: &'a [u8],
        header: ElementHeader,
        offset: u64,
    ) -> Result<Tracks<'a>> {
        let header_len = header.serialized_len();
        let payload = full.get(header_len..).unwrap_or(&[]);
        let mut entries: Vec<Element<'_>> = Vec::new();
        let mut pos = 0usize;
        while pos < payload.len() {
            match Element::parse_at(&payload[pos..], &self.limits) {
                Ok((Element::TrackEntry(entry), used)) => {
                    entries.push(Element::TrackEntry(entry));
                    pos = pos.saturating_add(used);
                }
                Ok((_other, used)) => pos = pos.saturating_add(used),
                Err(error) => {
                    let entry_offset = offset.saturating_add(
                        u64::try_from(header_len.saturating_add(pos)).unwrap_or(u64::MAX),
                    );
                    self.warned(Warning::UnparsedElement {
                        id: ids::TRACK_ENTRY,
                        offset: entry_offset,
                    });
                    let (entry_header, used) = match ElementHeader::parse(&payload[pos..]) {
                        Ok(parsed) => parsed,
                        Err(_) => return Err(error),
                    };
                    let Some(len) = entry_header.payload_len() else {
                        return Err(error);
                    };
                    let skip = used.saturating_add(usize::try_from(len).unwrap_or(usize::MAX));
                    if skip == 0 || pos.saturating_add(skip) > payload.len() {
                        break;
                    }
                    pos = pos.saturating_add(skip);
                }
            }
        }
        Ok(Tracks::from_parts(header, entries))
    }

    fn master_failed(&mut self, kind: MasterKind, offset: u64, error: Error) -> Result<()> {
        if !self.tolerated() {
            return Err(error);
        }
        let id = match kind {
            MasterKind::Ebml => crate::ebml::document::EBML_ID,
            MasterKind::SeekHead => ids::SEEK_HEAD,
            MasterKind::Info => ids::INFO,
            MasterKind::Tracks => ids::TRACKS,
            MasterKind::Cues => ids::CUES,
        };
        self.warned(Warning::UnparsedElement { id, offset });
        self.side = None;
        Ok(())
    }

    fn accept_info(&mut self, info: Option<&Info<'_>>, offset: u64, body: ByteRange) -> Result<()> {
        let Some(info) = info else {
            return Err(Error::UnexpectedElement {
                expected: ids::INFO,
                found: 0,
            });
        };
        let timestamp_scale_ns = info.timestamp_scale();
        if timestamp_scale_ns == 0 || i64::try_from(timestamp_scale_ns).is_err() {
            return Err(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "TimestampScale",
                reason: "must fit the non-zero signed tick arithmetic",
            }));
        }
        let summary = SegmentInfo {
            timestamp_scale_ns,
            duration: info.duration(),
        };
        self.timestamp_scale_ns = summary.timestamp_scale_ns;
        self.info = Some(summary);
        self.info_element = Some(ByteRange {
            offset,
            len: body.end().saturating_sub(offset),
        });
        Ok(())
    }

    fn accept_tracks(
        &mut self,
        tracks: Option<&Tracks<'_>>,
        offset: u64,
        body: ByteRange,
    ) -> Result<()> {
        let Some(tracks) = tracks else {
            return Err(Error::UnexpectedElement {
                expected: ids::TRACKS,
                found: 0,
            });
        };
        let capacity = usize::try_from(self.limits.max_tracks()).unwrap_or(usize::MAX);
        if tracks.entries().count() > capacity {
            return Err(Error::LimitExceeded {
                limit: "max_tracks",
                value: u64::try_from(tracks.entries().count()).unwrap_or(u64::MAX),
            });
        }
        self.tracks.clear();
        for entry in tracks.entries() {
            let video = entry.video();
            let audio = entry.audio();
            self.tracks.push(TrackInfo {
                track_number: entry.track_number().unwrap_or(0),
                track_uid: entry.track_uid().unwrap_or(0),
                track_type: entry.track_type(),
                track_type_value: entry
                    .find(ids::TRACK_TYPE)
                    .and_then(Element::as_unsigned)
                    .unwrap_or(0),
                codec_id: String::from(entry.codec_id().unwrap_or("")),
                codec_private: entry.codec_private().unwrap_or(&[]).to_vec(),
                default_duration_ns: entry.default_duration(),
                codec_delay_ns: entry.codec_delay(),
                seek_pre_roll_ns: entry.seek_pre_roll(),
                pixel_width: video.and_then(crate::matroska::Video::pixel_width),
                pixel_height: video.and_then(crate::matroska::Video::pixel_height),
                channels: audio.and_then(crate::matroska::Audio::channels),
                sampling_frequency: audio.and_then(crate::matroska::Audio::sampling_frequency),
            });
        }
        self.tracks_element = Some(ByteRange {
            offset,
            len: body.end().saturating_sub(offset),
        });
        self.tracks_parsed = true;
        Ok(())
    }

    fn accept_cues(
        &mut self,
        cues: Option<&crate::matroska::Cues<'_>>,
        offset: u64,
        body: ByteRange,
    ) -> Result<()> {
        let Some(cues) = cues else {
            return Err(Error::UnexpectedElement {
                expected: ids::CUES,
                found: 0,
            });
        };
        let capacity = usize::try_from(self.limits.max_cues()).unwrap_or(usize::MAX);
        self.cues.clear();
        for point in cues.points() {
            let Some(time) = point.time() else {
                continue;
            };
            let time = i64::try_from(time).map_err(|_| {
                Error::InvalidField(broadcast_core::error::InvalidField {
                    field: "CueTime",
                    reason: "does not fit a signed tick",
                })
            })?;
            for positions in point.track_positions() {
                let (Some(track), Some(cluster_position)) =
                    (positions.track(), positions.cluster_position())
                else {
                    continue;
                };
                if self.cues.len() >= capacity {
                    return Err(Error::LimitExceeded {
                        limit: "max_cues",
                        value: u64::try_from(self.cues.len()).unwrap_or(u64::MAX),
                    });
                }
                self.cues.push(CueEntry {
                    time,
                    track,
                    cluster_position,
                    duration: positions.duration(),
                });
            }
        }
        self.cues_parsed = true;
        let _ = offset;
        let _ = body;
        if let Some(side) = self.side.take() {
            self.pos = side.return_pos;
        }
        Ok(())
    }

    fn accept_seek_head(&mut self, seek_head: Option<&SeekHead<'_>>) -> Result<()> {
        let Some(seek_head) = seek_head else {
            return Err(Error::UnexpectedElement {
                expected: ids::SEEK_HEAD,
                found: 0,
            });
        };
        let capacity = usize::try_from(self.limits.max_children()).unwrap_or(usize::MAX);
        for seek in seek_head.seeks() {
            let (Some(id), Some(position)) = (seek.seek_id(), seek.seek_position()) else {
                continue;
            };
            if self.seek_entries.len() >= capacity {
                return Err(Error::LimitExceeded {
                    limit: "max_children",
                    value: u64::try_from(self.seek_entries.len()).unwrap_or(u64::MAX),
                });
            }
            self.seek_entries.push((fold_id(id), position));
        }
        if !matches!(self.source, Source::Seekable { .. })
            || self.cues_parsed
            || self.side.is_some()
        {
            return Ok(());
        }
        let Some(position) = self
            .seek_entries
            .iter()
            .find(|(id, _)| *id == ids::CUES)
            .map(|(_, position)| *position)
        else {
            return Ok(());
        };
        let target = self
            .segment_payload_start
            .checked_add(position)
            .ok_or(Error::SizeOverflow)?;
        self.side = Some(SideRead {
            return_pos: self.pos,
            header_seen: false,
        });
        self.pos = target;
        self.phase = Phase::Header { at: Where::Segment };
        Ok(())
    }

    fn accept_signed(
        &mut self,
        kind: SignedKind,
        offset: u64,
        body: ByteRange,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<()> {
        if self.short_supply(range, bytes) {
            return self.truncated(
                self.group_where(),
                offset,
                Error::Truncated {
                    what: "signed payload",
                    need: usize::try_from(body.len).unwrap_or(usize::MAX),
                    have: bytes.len(),
                },
            );
        }
        let value = crate::ebml::scalar::Signed::parse(bytes)?.value();
        match kind {
            SignedKind::DiscardPadding => {
                if let Some(group) = self.group.as_mut() {
                    group.discard_padding = Some(value);
                }
            }
        }
        self.pos = body.end();
        self.phase = Phase::Header {
            at: self.group_where(),
        };
        Ok(())
    }

    fn accept_unsigned(
        &mut self,
        kind: UnsignedKind,
        offset: u64,
        body: ByteRange,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<()> {
        if self.short_supply(range, bytes) {
            return self.truncated(
                Where::Segment,
                offset,
                Error::Truncated {
                    what: "unsigned payload",
                    need: usize::try_from(body.len).unwrap_or(usize::MAX),
                    have: bytes.len(),
                },
            );
        }
        let value = crate::ebml::scalar::Unsigned::parse(bytes)?.value();
        match kind {
            UnsignedKind::ClusterTimestamp => {
                let timestamp = i64::try_from(value).map_err(|_| {
                    Error::InvalidField(broadcast_core::error::InvalidField {
                        field: "Timestamp",
                        reason: "does not fit a signed tick",
                    })
                })?;
                self.cluster_timestamp = timestamp;
                if let Some(index) = self.current_cluster
                    && let Some(entry) = self.cluster_index.get_mut(index)
                {
                    entry.timestamp = timestamp;
                }
            }
            UnsignedKind::BlockDuration => {
                if let Some(group) = self.group.as_mut() {
                    group.duration = Some(value);
                }
            }
        }
        self.pos = body.end();
        self.phase = Phase::Header {
            at: match kind {
                UnsignedKind::ClusterTimestamp => Where::Cluster,
                UnsignedKind::BlockDuration => self.group_where(),
            },
        };
        Ok(())
    }

    fn accept_block(
        &mut self,
        owner: BlockOwner,
        header: ElementHeader,
        element: ByteRange,
        payload: ByteRange,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<()> {
        let parsed = self.parse_block(payload, bytes);
        match parsed {
            Ok((block_header, frames)) => {
                let relative = i64::from(block_header.relative_timestamp());
                let timestamp =
                    self.cluster_timestamp
                        .checked_add(relative)
                        .ok_or(Error::InvalidField(broadcast_core::error::InvalidField {
                            field: "Block timestamp",
                            reason: "cluster timestamp plus relative timestamp overflowed",
                        }))?;
                let pending = PendingFrames {
                    track: block_header.track(),
                    timestamp,
                    discardable: block_header.discardable(),
                    invisible: block_header.invisible(),
                    frames,
                };
                match owner {
                    BlockOwner::Cluster => {
                        let keyframe = block_header.keyframe_flag();
                        self.emit_frames(pending, keyframe, None, None)?;
                        self.pos = payload.end();
                        self.phase = Phase::Header { at: Where::Cluster };
                    }
                    BlockOwner::Group => {
                        if let Some(group) = self.group.as_mut() {
                            if group.block.is_some() {
                                return Err(Error::InvalidBlock {
                                    what: "BlockGroup carries two Blocks",
                                });
                            }
                            group.block = Some(pending);
                        }
                        self.pos = payload.end();
                        self.phase = Phase::Header {
                            at: self.group_where(),
                        };
                    }
                }
                let _ = (header, element, range);
                Ok(())
            }
            Err(error) => self.block_failed(owner, element.offset, payload, error),
        }
    }

    fn parse_block(
        &self,
        payload: ByteRange,
        bytes: &[u8],
    ) -> Result<(BlockHeader, Vec<ByteRange>)> {
        let (header, used) = BlockHeader::parse(bytes).map_err(|error| {
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) < payload.len {
                Error::LimitExceeded {
                    limit: "max_block_header_bytes",
                    value: payload.len,
                }
            } else {
                error
            }
        })?;
        let lace = LaceHeader::parse(
            bytes.get(used..).unwrap_or(&[]),
            header.lacing_kind(),
            usize::try_from(self.limits.max_lace_frames()).unwrap_or(usize::MAX),
        )
        .map_err(|error| {
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) < payload.len {
                Error::LimitExceeded {
                    limit: "max_block_header_bytes",
                    value: payload.len,
                }
            } else {
                error
            }
        })?;
        let data_len = payload
            .len
            .checked_sub(u64::try_from(used).map_err(|_| Error::SizeOverflow)?)
            .ok_or(Error::InvalidBlock {
                what: "Block header runs past the block",
            })?;
        let sizes = lace.frame_sizes(data_len)?;
        let mut offset = payload
            .offset
            .checked_add(u64::try_from(used).unwrap_or(u64::MAX))
            .and_then(|offset| offset.checked_add(u64::try_from(lace.data_offset()).ok()?))
            .ok_or(Error::SizeOverflow)?;
        let mut frames = Vec::with_capacity(sizes.len());
        for size in sizes {
            let end = offset.checked_add(size).ok_or(Error::SizeOverflow)?;
            frames.push(ByteRange { offset, len: size });
            offset = end;
        }
        Ok((header, frames))
    }

    fn block_failed(
        &mut self,
        owner: BlockOwner,
        offset: u64,
        payload: ByteRange,
        error: Error,
    ) -> Result<()> {
        if !self.tolerated() {
            return Err(error);
        }
        self.warned(Warning::MalformedBlock { offset });
        self.pos = payload.end();
        self.phase = Phase::Header {
            at: match owner {
                BlockOwner::Cluster => Where::Cluster,
                BlockOwner::Group => self.group_where(),
            },
        };
        Ok(())
    }

    fn emit_frames(
        &mut self,
        block: PendingFrames,
        keyframe: bool,
        duration: Option<u64>,
        discard_padding: Option<i64>,
    ) -> Result<()> {
        let count = u32::try_from(block.frames.len()).map_err(|_| Error::SizeOverflow)?;
        let timestamp_ns = i128::from(block.timestamp)
            .checked_mul(i128::from(self.timestamp_scale_ns))
            .ok_or(Error::InvalidField(broadcast_core::error::InvalidField {
                field: "Block timestamp",
                reason: "ticks times TimestampScale overflowed",
            }))?;
        for (index, frame) in block.frames.iter().enumerate() {
            let lace_index = u32::try_from(index).map_err(|_| Error::SizeOverflow)?;
            self.blocks.push_back(BlockStep {
                track: block.track,
                timestamp: block.timestamp,
                timestamp_ns,
                duration,
                keyframe,
                discardable: block.discardable,
                invisible: block.invisible,
                discard_padding,
                frame: *frame,
                lace_index,
                lace_count: count,
            });
        }
        Ok(())
    }

    fn accept_resync(
        &mut self,
        remaining: u64,
        at: Where,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<()> {
        let carry_len = self.resync_carry_len;
        let base = range
            .offset
            .saturating_sub(u64::try_from(carry_len).unwrap_or(0));
        let mut window = Vec::with_capacity(carry_len + bytes.len());
        window.extend_from_slice(&self.resync_carry[..carry_len]);
        window.extend_from_slice(bytes);
        if let Some(index) = find_resync(&window, at) {
            let target = base.saturating_add(u64::try_from(index).unwrap_or(0));
            self.pos = target;
            self.resync_carry_len = 0;
            self.phase = Phase::Header { at };
            return Ok(());
        }
        self.resync_carry_len = window.len().min(RESYNC_OVERLAP);
        let start = window.len() - self.resync_carry_len;
        self.resync_carry[..self.resync_carry_len].copy_from_slice(&window[start..]);
        let consumed = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.pos = range.end();
        let left = remaining.saturating_sub(consumed);
        if left == 0 || self.short_supply(range, bytes) {
            return self.end_resync(at);
        }
        self.phase = Phase::Resync {
            remaining: left,
            at,
        };
        Ok(())
    }
}

/// Build one element's bytes from its header and a payload already supplied.
fn element_bytes(header: ElementHeader, payload: &[u8]) -> Result<Vec<u8>> {
    let header_len = header.serialized_len();
    let mut full = alloc::vec![0u8; header_len + payload.len()];
    let used = header.write(&mut full)?;
    let target = full.get_mut(used..).ok_or(Error::SizeOverflow)?;
    target.copy_from_slice(payload);
    Ok(full)
}

/// Fold a `SeekID` payload into the element id it names.
fn fold_id(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for octet in bytes.iter().take(MAX_ELEMENT_ID_OCTETS) {
        value = (value << u8::BITS) | u32::from(*octet);
    }
    value
}

/// Whether `id` starts a new level-1 element, ending an unknown-size Cluster
/// (RFC 8794 §6.2).
fn is_level_one(id: u32) -> bool {
    id == ids::SEGMENT || ids::SEGMENT_CHILD_IDS.contains(&id)
}

/// Find the first element id a resync at `at` may land on.
///
/// At the Segment level that is a level-1 id, as before. Inside a Cluster it
/// is a level-1 id (which ends the Cluster) or a known Cluster child, so a
/// damaged Cluster child does not lose the rest of its Cluster. Every candidate
/// must also parse as a complete element header, so a byte pattern inside a
/// Block payload is not mistaken for a boundary.
fn find_resync(window: &[u8], at: Where) -> Option<usize> {
    for index in 0..window.len() {
        let rest = window.get(index..)?;
        let Some((header, _)) = ElementHeader::parse(rest).ok() else {
            continue;
        };
        let id = header.id().value();
        let boundary = if matches!(at, Where::Cluster | Where::Group { .. }) {
            is_level_one(id) || is_cluster_child(id)
        } else {
            is_level_one(id)
        };
        if boundary {
            return Some(index);
        }
    }
    None
}

/// The Cluster's direct children this reader can walk, plus the global
/// elements every level may carry.
fn is_cluster_child(id: u32) -> bool {
    matches!(
        id,
        ids::TIMESTAMP
            | ids::SIMPLE_BLOCK
            | ids::BLOCK_GROUP
            | ids::POSITION
            | ids::PREV_SIZE
            | ids::ENCRYPTED_BLOCK
            | ids::SILENT_TRACKS
    ) || id == crate::ebml::global::VOID_ID
        || id == crate::ebml::global::CRC32_ID
}

impl Reader {
    /// Seek to the last cue point (or, without Cues, the last indexed Cluster)
    /// at or before `timestamp` for `track`.
    ///
    /// `timestamp` is in Matroska Ticks (RFC 9559 §11) and `track` is a
    /// `TrackNumber`. Cues are used when the file has them; otherwise the
    /// Cluster index the walk recorded is binary-searched, which is bounded by
    /// [`Limits::max_clusters`]. The walk then resumes at that Cluster header
    /// and the caller steps forward: for a Cues seek the Cluster starts on the
    /// requested track's preceding keyframe (RFC 9559 §20).
    pub fn seek_to(&mut self, track: u64, timestamp: i64) -> Result<SeekOutcome> {
        if !self.segment_started {
            return Err(Error::SeekBeforeSegment);
        }
        let mut cue: Option<CueEntry> = None;
        for entry in &self.cues {
            if entry.track == track && entry.time <= timestamp {
                cue = Some(*entry);
            }
        }
        if let Some(cue) = cue {
            let offset = self
                .segment_payload_start
                .checked_add(cue.cluster_position)
                .ok_or(Error::SizeOverflow)?;
            self.position_at(offset)?;
            return Ok(SeekOutcome {
                offset,
                timestamp: cue.time,
                method: SeekMethod::Cues,
            });
        }
        if self.cluster_index.is_empty() {
            return Err(Error::NoIndex { track });
        }
        let mut chosen: Option<ClusterIndexEntry> = None;
        for entry in &self.cluster_index {
            if entry.timestamp <= timestamp {
                chosen = Some(*entry);
            } else {
                break;
            }
        }
        let entry = chosen.unwrap_or(self.cluster_index[0]);
        self.position_at(entry.offset)?;
        Ok(SeekOutcome {
            offset: entry.offset,
            timestamp: entry.timestamp,
            method: SeekMethod::ClusterScan,
        })
    }

    fn position_at(&mut self, offset: u64) -> Result<()> {
        if let Source::ForwardOnly = self.source
            && offset < self.highest_requested
        {
            return Err(Error::BackwardSeek {
                offset,
                position: self.highest_requested,
            });
        }
        self.pos = offset;
        self.pending = None;
        self.blocks.clear();
        self.elements.clear();
        self.held = None;
        self.side = None;
        self.group = None;
        self.cluster_end = None;
        self.current_cluster = None;
        self.cluster_timestamp = 0;
        self.done = false;
        self.phase = Phase::Header { at: Where::Segment };
        Ok(())
    }
}
