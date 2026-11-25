//! The sans-IO MP4 reader: it asks the caller for structure bytes and yields
//! every track, then one sample descriptor at a time.
//!
//! The reader owns the bytes of one `moov` or one `moof` at a time and nothing
//! else. Sample data never passes through it: a [`SampleRef`] says where the
//! bytes are, and the caller fetches that range itself. A progressive file
//! whose `moov` follows a multi-gigabyte `mdat` is reached by skipping the
//! media box by its declared size, so the walk costs only box headers; over a
//! forward-only source that file is a typed
//! [`Error::MoovAfterMdatForwardOnly`].
//!
//! A fragmented file (a `moov` with `mvex`) is walked one `moof` at a time:
//! the fragment's box is read under [`Limits::max_fragment_bytes`], its `trex`
//! defaults applied, and the following `mdat` payload ranges learned from their
//! headers. [`Step::Fragment`] followed by one [`Step::Sample`] per sample is
//! emitted as soon as every run of the fragment's `traf`s resolves inside the
//! payloads located so far, before the walk asks for anything past that media,
//! so a forward-only source still has the sample bytes ahead of it. A run that
//! addresses a later `mdat` keeps the fragment pending until that box is
//! located. Fragment sample data is addressed by [`crate::frag_offsets`] with
//! those ranges, never with a slice of the file, so no byte inside an `mdat`
//! payload is ever requested.
//!
//! Driving is [`Reader::step`] / [`Reader::supply`]: each `step` returns either
//! a [`Step::Need`] range to supply or a typed step, and `supply` accepts
//! exactly the outstanding range. See the streaming-first design §4.2 and §4.4
//! (`docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md`)
//! and ISO/IEC 14496-12:2026 §4.2, §8.2-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use alloc::vec::Vec;
use core::cmp::Ordering;

use broadcast_common::Parse;
use broadcast_common::timescale::Timescale;

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, BoxHeader, BoxIter, FULLBOX_EXTRA_SIZE,
    parse_box,
};
use crate::cenc::{SampleEncryptionBox, TrackEncryptionBox, sinf_of_sample_entry};
use crate::error::{Error, Result};
use crate::frag_offsets::{MdatRange, sample_ranges_in_run, sample_ranges_in_run_until};
use crate::init_segment::{
    MediaBox, MovieBox, SampleEntryVariant, StblChild, TrackBox, TrackExtendsBox,
};
use crate::limits::Limits;
use crate::metadata::MovieMetadata;
use crate::movie_fragment::{
    MAX_ZERO_WIDTH_TRUN_SAMPLES, MovieFragmentBox, SAMPLE_FLAG_IS_NON_SYNC, TrackFragmentBox,
    TrafChild, apply_trex_defaults,
};
use crate::plan::{BoxPosition, SamplePlan, TopLevelBox, sample_description, sample_table};
use crate::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox};
use crate::sample_table::{CursorState, SampleRef};
use crate::segments::FileTypeBox;
use crate::timing::{
    EditListEntry, MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox,
    SegmentIndexBox,
};
use crate::track_id::TrackId;
use crate::track_reference::TrackReferenceTypeBox;
use crate::writer::SampleAux;

/// Largest bytes a top-level box header can occupy: 32-bit `size`, 32-bit
/// `type`, a 64-bit `largesize` and a 16-byte `uuid` usertype
/// (ISO/IEC 14496-12:2026 §4.2).
const MAX_BOX_HEADER_BYTES: u64 = 32;

/// A box header occupies at least `size` + `type` (§4.2).
const MIN_BOX_HEADER_BYTES: u64 = BOX_HEADER_MIN_SIZE as u64;

/// A sample description starts with a FullBox header then a 32-bit entry count
/// (ISO/IEC 14496-12:2026 §8.5.2).
const STSD_ENTRY_COUNT_OFFSET: usize = FULLBOX_EXTRA_SIZE;

/// Depth at which the `moov` walk starts counting: the movie box is one.
const MOOV_DEPTH: u32 = 1;
/// Depth of a `trak` inside `moov`.
const TRAK_DEPTH: u32 = MOOV_DEPTH + 1;
/// Depth of a `mdia` inside `trak`.
const MDIA_DEPTH: u32 = TRAK_DEPTH + 1;
/// Depth of a `minf` inside `mdia`.
const MINF_DEPTH: u32 = MDIA_DEPTH + 1;
/// Depth of a `stbl` inside `minf`.
const STBL_DEPTH: u32 = MINF_DEPTH + 1;
/// Depth of a `stsd` inside `stbl`.
const STSD_DEPTH: u32 = STBL_DEPTH + 1;

/// Bytes of a `mfro` box, header included: the trailer a reader looks at to
/// find a trailing `mfra` (ISO/IEC 14496-12:2026 §8.8.11).
const MFRO_BOX_LEN: u64 = 16;

/// Indexes the reader keeps from the walk: one per track of a segment, for the
/// `sidx` that precedes each track's fragments.
const MAX_REMEMBERED_INDEXES: usize = 16;

/// A half-open byte range in the input file: `[offset, offset + len)`.
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

/// The order [`Reader::step`] yields [`Step::Sample`] steps in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SampleOrder {
    /// Merge tracks by decode time, ties broken by `moov` track order.
    #[default]
    DecodeTime,
    /// Ascending file offset, smallest next offset among the track heads.
    File,
}

/// Whether the caller's byte source can seek backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Source {
    /// Every requested range can be served, including a backward seek.
    #[default]
    Seekable,
    /// Ranges must not move backwards.
    ForwardOnly,
}

/// How strictly the reader treats known real-world faults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// Any violation is an error.
    #[default]
    Strict,
    /// The faults listed by the design §4.7 are reported as warnings.
    Permissive,
}

/// Configuration for [`Reader::new`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ReaderConfig {
    limits: Limits,
    total_len: Option<u64>,
    order: SampleOrder,
    source: Source,
    mode: Mode,
    registry: Option<crate::registry::BoxRegistry>,
}

impl ReaderConfig {
    /// Set the total input length, which lets a box of size 0 run to the end.
    #[must_use]
    pub fn with_total_len(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// Set the sample delivery order.
    #[must_use]
    pub fn with_order(mut self, order: SampleOrder) -> Self {
        self.order = order;
        self
    }

    /// Set whether the source can seek backwards.
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// Set the fault policy.
    #[must_use]
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the hostile-input caps.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
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

    /// The sample delivery order.
    #[must_use]
    pub fn order(&self) -> SampleOrder {
        self.order
    }

    /// Whether the source can seek backwards.
    #[must_use]
    pub fn source(&self) -> Source {
        self.source
    }

    /// The fault policy.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Attach a [`BoxRegistry`](crate::registry::BoxRegistry) of private box
    /// types, so a registered `stsd` entry comes back typed in
    /// [`SampleEntryInfo::registered`].
    #[must_use]
    pub fn with_registry(mut self, registry: crate::registry::BoxRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// The attached private box registry, when one was set.
    #[must_use]
    pub fn registry(&self) -> Option<&crate::registry::BoxRegistry> {
        self.registry.as_ref()
    }
}

/// One sample description (`stsd` entry, ISO/IEC 14496-12:2026 §8.5.2).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SampleEntryInfo {
    /// The entry's four-CC.
    pub fourcc: [u8; BOX_TYPE_SIZE],
    /// The full sample-entry box bytes, header included, copied from `moov`.
    pub body: Vec<u8>,
    /// The typed parse, or `None` when the entry is an unknown four-CC.
    pub typed: Option<SampleEntryVariant>,
    /// The [`ReaderConfig`]'s private-box-registry parse of this entry, when a
    /// registration matched its four-CC; call
    /// [`RegisteredEntry::downcast_ref`](crate::registry::RegisteredEntry::downcast_ref)
    /// for the concrete type.
    pub registered: Option<crate::registry::RegisteredEntry>,
}

/// Every fact the reader surfaces about one track, including tracks the media
/// IR has no mapping for.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TrackInfo {
    /// The `tkhd.track_ID` (§8.3.2).
    pub track_id: TrackId,
    /// The `hdlr.handler_type` (§8.4.3).
    pub handler: [u8; BOX_TYPE_SIZE],
    /// The `mdhd.timescale` (§8.4.2).
    pub timescale: Timescale,
    /// The `mdhd.duration`, in the track timescale (§8.4.2).
    pub duration: u64,
    /// The full track header fields (§8.3.2).
    pub header: crate::init_segment::TrackHeaderBox,
    /// The `tref` references (§8.3.3), preserving wire order.
    pub references: Vec<TrackReferenceTypeBox>,
    /// The `elst` entries, unapplied (§8.6.6).
    pub edits: Vec<EditListEntry>,
    /// The `stsd` entries in wire order; `description_index` on a sample is
    /// one-based into this list.
    pub entries: Vec<SampleEntryInfo>,
    /// The QuickTime track aperture dimensions (`tapt`), when present.
    pub tapt: Option<crate::quicktime::TrackApertureModeDimensionsBox>,
}

impl TrackInfo {
    /// The reader's track view for an already-parsed `trak`.
    ///
    /// A caller that holds the whole `moov` (the whole-input demuxers) needs
    /// the same per-track facts [`Reader`] surfaces in [`Step::Tracks`]. This
    /// is that view, built from the typed box tree: each `stsd` entry is
    /// serialised back to its own box bytes so [`SampleEntryInfo::body`] is
    /// the bytes the reader would have copied. `handler` is zero when the
    /// `trak` carries no `hdlr`, which the whole-input demuxers never read.
    pub fn from_trak(trak: &TrackBox) -> Result<Self> {
        use broadcast_common::Serialize;
        let mdia = trak
            .mdia
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
        let mdhd = mdia
            .mdhd
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "mdhd" })?;
        let minf = mdia
            .minf
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "minf" })?;
        let stbl = minf
            .stbl
            .as_ref()
            .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
        let stsd = stbl
            .children
            .iter()
            .find_map(|child| match child {
                StblChild::Stsd(stsd) => Some(Ok(stsd.clone())),
                StblChild::Opaque(bytes)
                    if bytes.len() >= BOX_HEADER_MIN_SIZE
                        && &bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] == b"stsd" =>
                {
                    Some(crate::init_segment::SampleDescriptionBox::parse(
                        bytes.as_slice(),
                    ))
                }
                _ => None,
            })
            .ok_or(Error::UnexpectedBox { expected: "stsd" })??;
        let mut entries = Vec::with_capacity(stsd.entries.len());
        for entry in &stsd.entries {
            let mut body = alloc::vec![0u8; entry.serialized_len()];
            let len = entry.serialize_into(&mut body)?;
            body.truncate(len);
            let mut fourcc = [0u8; BOX_TYPE_SIZE];
            if body.len() >= BOX_HEADER_MIN_SIZE {
                fourcc.copy_from_slice(&body[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
            }
            entries.push(SampleEntryInfo {
                fourcc,
                body,
                typed: match entry {
                    SampleEntryVariant::Unknown(_) => None,
                    known => Some(known.clone()),
                },
                registered: None,
            });
        }
        let references = match trak.track_references()? {
            Some(reference) => reference.references,
            None => Vec::new(),
        };
        let edits = match trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()) {
            Some(elst) => elst.entries.clone(),
            None => Vec::new(),
        };
        let tapt = trak
            .opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::aperture::TAPT)
            .and_then(|b| {
                crate::quicktime::TrackApertureModeDimensionsBox::parse_body(&b.data).ok()
            });
        Ok(Self {
            track_id: trak.tkhd.track_id,
            handler: mdia
                .hdlr
                .as_ref()
                .map_or([0u8; BOX_TYPE_SIZE], |hdlr| hdlr.handler_type),
            timescale: mdhd.timescale,
            duration: mdhd.duration,
            header: trak.tkhd.clone(),
            references,
            edits,
            entries,
            tapt,
        })
    }
}

/// One fragment (`moof`) the reader is about to hand out samples for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FragmentInfo {
    /// The `mfhd.sequence_number` (§8.8.2).
    pub sequence_number: u32,
    /// The `moof` box's own byte range, header included.
    pub moof: ByteRange,
    /// The tracks this fragment carries samples for, in `traf` order.
    pub tracks: Vec<TrackId>,
}

/// A fault [`Mode::Permissive`] tolerated, reported once as [`Step::Warning`].
///
/// Each variant names the box or sample table row that carried the fault and
/// the absolute file offset it starts at. A warning never invents a field
/// value: the reader keeps what the file states and drops only what it cannot
/// address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Warning {
    /// A box inside `moov` declared a size past its parent; it was clamped to
    /// the parent's end.
    ClampedBox {
        /// The four-CC of the clamped box.
        box_type: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the box.
        offset: u64,
    },
    /// A known sample-entry four-CC did not parse; the entry was kept raw, with
    /// no typed form.
    UnparseableSampleEntry {
        /// The original four-CC.
        fourcc: [u8; BOX_TYPE_SIZE],
        /// Absolute offset of the entry.
        offset: u64,
    },
    /// An `mp4a` sound entry's `wave` atom carried no `esds`.
    WaveWithoutEsds {
        /// Absolute offset of the `wave` atom.
        offset: u64,
    },
    /// The movie header timescale was zero; it was treated as 1 so the walk can
    /// continue. Durations in the movie timescale are unknown.
    ZeroMovieTimescale {
        /// Absolute offset of the `mvhd` box.
        offset: u64,
    },
    /// The final `mdat` was truncated; samples fully inside the input were
    /// emitted.
    TruncatedMdat {
        /// Absolute offset of the first range or box past the input end.
        offset: u64,
    },
    /// A final fragment (`moof` or its `mdat`) was truncated.
    TruncatedFragment {
        /// Absolute offset of the truncated fragment box.
        offset: u64,
    },
    /// A `moof` in the middle of the file was malformed; the walk continued
    /// with the next fragment.
    MalformedFragment {
        /// Absolute offset of the malformed `moof`.
        offset: u64,
    },
}

impl Warning {
    /// The warning's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::ClampedBox { .. } => "clamped-box",
            Self::UnparseableSampleEntry { .. } => "unparseable-sample-entry",
            Self::WaveWithoutEsds { .. } => "wave-without-esds",
            Self::ZeroMovieTimescale { .. } => "zero-movie-timescale",
            Self::TruncatedMdat { .. } => "truncated-mdat",
            Self::TruncatedFragment { .. } => "truncated-fragment",
            Self::MalformedFragment { .. } => "malformed-fragment",
        }
    }
}

broadcast_common::impl_spec_display!(Warning);

impl Warning {
    /// The absolute file offset the fault starts at.
    #[must_use]
    pub const fn position(&self) -> u64 {
        match self {
            Self::ClampedBox { offset, .. }
            | Self::UnparseableSampleEntry { offset, .. }
            | Self::WaveWithoutEsds { offset }
            | Self::ZeroMovieTimescale { offset }
            | Self::TruncatedMdat { offset }
            | Self::TruncatedFragment { offset }
            | Self::MalformedFragment { offset } => *offset,
        }
    }
}

/// One step of the reader state machine.
#[derive(Debug)]
#[non_exhaustive]
pub enum Step<'a> {
    /// Supply exactly this range with [`Reader::supply`].
    Need(ByteRange),
    /// Every track of the movie, in `moov` order. Emitted once.
    Tracks(&'a [TrackInfo]),
    /// The movie's and each track's typed metadata. Emitted once, after
    /// [`Step::Tracks`], only when the file carries `udta` or `meta`.
    Metadata(&'a MovieMetadata),
    /// A fragment's samples follow.
    Fragment(FragmentInfo),
    /// One sample descriptor, in the configured order.
    Sample(SampleRef),
    /// A fault the configured [`Mode::Permissive`] tolerated.
    Warning(Warning),
    /// No more steps.
    Done,
}

/// The outstanding byte demand.
#[derive(Debug, Clone, Copy)]
enum Pending {
    Header(ByteRange),
    Ftyp(ByteRange),
    Moov(ByteRange),
    Moof(ByteRange),
    Index(ByteRange),
    MfraTrailer(ByteRange),
    Mfra(ByteRange),
    TopLevel(ByteRange),
}

impl Pending {
    fn range(&self) -> ByteRange {
        match self {
            Self::Header(range)
            | Self::Ftyp(range)
            | Self::Moov(range)
            | Self::Moof(range)
            | Self::Index(range)
            | Self::MfraTrailer(range)
            | Self::Mfra(range)
            | Self::TopLevel(range) => *range,
        }
    }
}

/// One track's cursor and the `moov` track index it was built from.
#[derive(Debug, Clone)]
struct CursorSlot {
    track_index: usize,
    state: CursorState,
}

/// A raw `stsd` entry taken from the supplied `moov` bytes.
struct RawEntry {
    fourcc: [u8; BOX_TYPE_SIZE],
    body: Vec<u8>,
    offset: u64,
}

/// One protected track's `tenc` defaults, resolved from the `sinf` of its
/// sample entry (ISO/IEC 23001-7 §12.2).
#[derive(Debug, Clone)]
struct TrackCrypto {
    tenc: TrackEncryptionBox,
}

/// The `moof` being read, with the `mdat` payload ranges its runs may address.
#[derive(Debug)]
struct FragmentState {
    offset: u64,
    range: ByteRange,
    moof: Option<MovieFragmentBox>,
    mdats: Vec<MdatRange>,
}

/// A `sidx` found in the walk, with the byte offset its `first_offset` is
/// measured from (the first byte after the box, §8.14.3).
#[derive(Debug)]
struct SegmentIndex {
    index: SegmentIndexBox,
    anchor: u64,
}

/// Where a fragmented seek is in its byte-driven search for an index.
#[derive(Debug, Clone, Copy)]
enum SeekPhase {
    /// Walking headers for a `sidx` that precedes the fragments.
    Scanning,
    /// Waiting for the trailing `mfro` bytes at the end of the file.
    Trailer,
    /// Waiting for the `mfra` box the `mfro` pointed at.
    Mfra,
}

/// A seek that needs index bytes before it can move the walk.
#[derive(Debug, Clone, Copy)]
struct SeekRequest {
    track: TrackId,
    decode_time: u64,
    phase: SeekPhase,
}

/// One owned step of a resolved fragment, so handing it out does not keep the
/// reader borrowed.
#[derive(Debug)]
enum FragmentStep {
    Fragment(FragmentInfo),
    Sample(SampleRef),
}

/// One fragment resolved against the `mdat` payloads located so far, before
/// the reader's per-track cursors are advanced.
#[derive(Debug)]
struct ResolvedFragment {
    info: FragmentInfo,
    samples: Vec<SampleRef>,
    truncated: bool,
    /// Per track: the `moov` track index, the decode time after the fragment
    /// and the sample count after it.
    cursors: Vec<(usize, u64, u32)>,
}

/// What the walk does with a fragment whose runs do not all resolve inside the
/// `mdat` payloads located so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unresolved {
    /// A `mdat` later in the file may still hold the data: leave the fragment
    /// pending and keep walking.
    Keep,
    /// Nothing more can arrive for it, so the resolver's own error is the
    /// caller's.
    Fail,
}

/// A fragment whose samples are resolved and waiting to be handed out.
#[derive(Debug)]
struct ReadyFragment {
    info: FragmentInfo,
    samples: Vec<SampleRef>,
    next: usize,
    emitted: bool,
}

/// The sans-IO MP4 reader. See the [module docs](self).
#[derive(Debug)]
pub struct Reader {
    config: ReaderConfig,
    pending: Option<Pending>,
    pos: u64,
    highest_requested: u64,
    saw_mdat: bool,
    movie: Option<MovieBox>,
    brand: Option<FileTypeBox>,
    warnings: Vec<Warning>,
    deferred_warning: Option<Warning>,
    mdat_truncation_reported: bool,
    tracks: Vec<TrackInfo>,
    cursors: Vec<CursorSlot>,
    emitted_tracks: bool,
    metadata: Option<MovieMetadata>,
    emitted_metadata: bool,
    done: bool,
    fragmented: bool,
    trex: Vec<TrackExtendsBox>,
    crypto: Vec<Option<TrackCrypto>>,
    decode_times: Vec<u64>,
    sample_counts: Vec<u32>,
    fragment: Option<FragmentState>,
    top_level: Vec<TopLevelBox>,
    ready: Option<ReadyFragment>,
    last_fragment: Option<(u64, Vec<SampleRef>)>,
    open_box: Option<ByteRange>,
    indexes: Vec<SegmentIndex>,
    seek: Option<SeekRequest>,
    #[cfg(feature = "scan-counters")]
    bytes_requested: u64,
}

impl Reader {
    /// Create a reader. It issues its first [`Step::Need`] from the first
    /// [`Reader::step`].
    #[must_use]
    pub fn new(config: ReaderConfig) -> Self {
        Self {
            config,
            pending: None,
            pos: 0,
            highest_requested: 0,
            saw_mdat: false,
            movie: None,
            brand: None,
            warnings: Vec::new(),
            deferred_warning: None,
            mdat_truncation_reported: false,
            tracks: Vec::new(),
            cursors: Vec::new(),
            emitted_tracks: false,
            metadata: None,
            emitted_metadata: false,
            done: false,
            fragmented: false,
            trex: Vec::new(),
            crypto: Vec::new(),
            decode_times: Vec::new(),
            sample_counts: Vec::new(),
            fragment: None,
            top_level: Vec::new(),
            ready: None,
            last_fragment: None,
            open_box: None,
            indexes: Vec::new(),
            seek: None,
            #[cfg(feature = "scan-counters")]
            bytes_requested: 0,
        }
    }

    /// Supply the outstanding [`Step::Need`] range.
    ///
    /// `offset` must equal the requested offset and `bytes` may not be longer
    /// than the requested range; a shorter supply means the input ended there.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Err(Error::InvalidInput("supply without an outstanding need"));
        };
        let expected = pending.range();
        let got = ByteRange {
            offset,
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        };
        if offset != expected.offset || got.len > expected.len {
            self.pending = Some(pending);
            return Err(Error::SupplyMismatch { expected, got });
        }
        #[cfg(feature = "scan-counters")]
        {
            self.bytes_requested = self.bytes_requested.saturating_add(got.len);
        }
        match pending {
            Pending::Header(range) => self.accept_header(range, bytes),
            Pending::Ftyp(_) => self.accept_ftyp(bytes),
            Pending::Moov(range) => self.accept_moov(range, bytes),
            Pending::Moof(_) => self.accept_moof(bytes),
            Pending::Index(range) => self.accept_index(range, bytes),
            Pending::MfraTrailer(_) => self.accept_mfra_trailer(bytes),
            Pending::Mfra(_) => self.accept_mfra(bytes),
            Pending::TopLevel(_) => self.accept_top_level(bytes),
        }
    }

    /// Bytes the reader has asked the caller to supply, under the
    /// `scan-counters` feature.
    #[cfg(feature = "scan-counters")]
    #[must_use]
    pub fn bytes_requested(&self) -> u64 {
        self.bytes_requested
    }

    /// Seek to the last sync sample at or before `decode_time`, expressed in
    /// `track`'s timescale.
    ///
    /// A progressive file seeks every track's cursor through its `stss`
    /// (§8.6.2). A fragmented file positions the walk on the fragment holding
    /// the time, using a `sidx` seen before the fragments (§8.14.3) or, failing
    /// that, the trailing `mfra` reached through its `mfro` (§8.8.9-§8.8.11);
    /// a file with neither is [`Error::NoIndex`]. When index bytes must be read
    /// first, this returns with a [`Step::Need`] outstanding and the next
    /// material step is the [`Step::Fragment`] containing the time.
    pub fn seek_to(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        if self.movie.is_none() {
            return Err(Error::InvalidInput("seek before moov is parsed"));
        }
        let named = self
            .tracks
            .iter()
            .position(|info| info.track_id == track)
            .ok_or(Error::InvalidInput("unknown track"))?;
        if self.fragmented {
            return self.seek_fragmented(track, decode_time);
        }
        let movie = self
            .movie
            .as_ref()
            .ok_or(Error::InvalidInput("seek before moov is parsed"))?;
        let from_timescale = self.tracks[named].timescale;
        for slot in &mut self.cursors {
            let info = self
                .tracks
                .get(slot.track_index)
                .ok_or(Error::InvalidInput("unknown track"))?;
            let target = rescale_floor(decode_time, from_timescale, info.timescale);
            let trak = movie
                .tracks
                .get(slot.track_index)
                .ok_or(Error::UnexpectedBox { expected: "trak" })?;
            let stbl = sample_table(trak).ok_or(Error::NoIndex {
                track: info.track_id,
            })?;
            slot.state.seek_sync(stbl, target)?;
        }
        self.done = false;
        Ok(())
    }

    /// Declare that the bytes supplied so far end one self-contained unit of
    /// sequential input: a media segment.
    ///
    /// `unit_end` is the absolute offset one past the unit's last byte, the
    /// caller's running sequential position. A top-level box whose declared end
    /// lies past it is dropped. In [`Mode::Permissive`] the cut fragment's
    /// samples wholly inside the unit stay available and a
    /// [`Warning::TruncatedFragment`] follows them; in [`Mode::Strict`] the
    /// truncation is the typed error. The `moov` and its track state, the
    /// `trex` defaults and the running decode time are kept, and the top-level
    /// walk resumes at `unit_end`, so the next segment is read from its first
    /// byte. A unit that ended on a box boundary produces no step.
    pub fn end_of_unit(&mut self, unit_end: u64) -> Result<()> {
        let overrun = self.open_box.is_some_and(|range| range.end() > unit_end);
        let samples_overrun =
            self.ready
                .as_ref()
                .is_some_and(|ready| ready.samples.iter().any(|sample| extent(sample) > unit_end))
                || self.last_fragment.as_ref().is_some_and(|(_, samples)| {
                    samples.iter().any(|sample| extent(sample) > unit_end)
                });
        let truncated = overrun || samples_overrun;
        if truncated && !self.tolerated() {
            let (start, end) = match self.open_box.filter(|range| range.end() > unit_end) {
                Some(range) => (range.offset, range.end()),
                None => (unit_end, unit_end),
            };
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: usize::try_from(end.saturating_sub(start)).unwrap_or(usize::MAX),
                    have: usize::try_from(unit_end.saturating_sub(start)).unwrap_or(usize::MAX),
                    what: "top-level box body",
                },
            ));
        }
        if let Some(ready) = self.ready.take() {
            self.retime_after_cut(&ready.samples, unit_end);
            let emitted = ready.next;
            let samples: Vec<SampleRef> = ready
                .samples
                .into_iter()
                .enumerate()
                .filter(|(index, sample)| *index < emitted || extent(sample) <= unit_end)
                .map(|(_, sample)| sample)
                .collect();
            let kept = samples.len();
            if !ready.emitted || emitted < kept {
                self.ready = Some(ReadyFragment {
                    info: ready.info,
                    samples,
                    next: emitted.min(kept),
                    emitted: ready.emitted,
                });
            } else {
                self.last_fragment = Some((ready.info.moof.offset, samples));
            }
        } else if let Some((offset, samples)) = self.last_fragment.take() {
            self.retime_after_cut(&samples, unit_end);
            self.last_fragment = Some((offset, samples));
        }
        if truncated {
            self.deferred_warning = Some(Warning::TruncatedFragment {
                offset: self
                    .fragment
                    .as_ref()
                    .map(|fragment| fragment.offset)
                    .or_else(|| self.ready.as_ref().map(|ready| ready.info.moof.offset))
                    .or_else(|| self.last_fragment.as_ref().map(|(offset, _)| *offset))
                    .unwrap_or(unit_end),
            });
        }
        self.fragment = None;
        self.open_box = None;
        self.last_fragment = None;
        self.pending = None;
        self.pos = unit_end;
        self.highest_requested = unit_end;
        self.done = false;
        Ok(())
    }

    /// Recompute each track's running decode time and sample count from the
    /// samples that survive a cut, so a following fragment without `tfdt`
    /// continues from the last kept sample rather than from a dropped one.
    fn retime_after_cut(&mut self, samples: &[SampleRef], unit_end: u64) {
        let mut base: Vec<(usize, u64, u32)> = Vec::new();
        let mut advanced: Vec<(usize, u64, u32)> = Vec::new();
        for sample in samples {
            let Some(index) = self
                .tracks
                .iter()
                .position(|info| info.track_id == sample.track)
            else {
                continue;
            };
            match base.iter_mut().find(|slot| slot.0 == index) {
                Some(slot) => {
                    slot.1 = slot.1.min(sample.decode_time);
                    slot.2 = slot.2.min(sample.index);
                }
                None => base.push((index, sample.decode_time, sample.index)),
            }
            if extent(sample) <= unit_end {
                let decode_time = sample
                    .decode_time
                    .saturating_add(u64::from(sample.duration));
                let count = sample.index.saturating_add(1);
                match advanced.iter_mut().find(|slot| slot.0 == index) {
                    Some(slot) => {
                        slot.1 = decode_time;
                        slot.2 = count;
                    }
                    None => advanced.push((index, decode_time, count)),
                }
            }
        }
        for (index, decode_time, count) in base {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
        for (index, decode_time, count) in advanced {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
    }

    /// The file's `ftyp` brand box, when the file carries one.
    ///
    /// A QuickTime `.mov` whose major brand is `qt  ` and a brandless file both
    /// read completely; this is how a caller tells them from an ISO file.
    #[must_use]
    pub fn brand(&self) -> Option<&FileTypeBox> {
        self.brand.as_ref()
    }

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<SamplePlan<'_>> {
        self.movie.as_ref().map(|movie| {
            SamplePlan::new(movie, &self.tracks, self.config.total_len, &self.top_level)
        })
    }

    /// The movie timescale (`mvhd.timescale`), once `moov` is parsed.
    #[must_use]
    pub fn movie_timescale(&self) -> Option<Timescale> {
        self.movie.as_ref().map(|movie| movie.mvhd.timescale)
    }

    /// Whether the movie is fragmented (`moov` carries `mvex`), once `moov` is
    /// parsed. `None` before the track table is known.
    #[must_use]
    pub fn is_fragmented(&self) -> Option<bool> {
        self.movie.as_ref().map(|_| self.fragmented)
    }
}

impl Reader {
    fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Report a truncated final `mdat` once per file: the box header and a
    /// sample that runs past the input describe the same fault.
    fn warn_truncated_mdat(&mut self, offset: u64) {
        if self.mdat_truncation_reported {
            return;
        }
        self.mdat_truncation_reported = true;
        self.warn(Warning::TruncatedMdat { offset });
    }

    fn finish_step(&mut self) -> Result<Step<'_>> {
        self.done = true;
        if let Some(warning) = self.next_warning() {
            return Ok(Step::Warning(warning));
        }
        Ok(Step::Done)
    }

    fn take_warning(&mut self) -> Option<Warning> {
        if self.warnings.is_empty() {
            return None;
        }
        Some(self.warnings.remove(0))
    }

    fn next_warning(&mut self) -> Option<Warning> {
        if self.ready.is_some() {
            return None;
        }
        self.deferred_warning.take().or_else(|| self.take_warning())
    }

    fn tolerated(&self) -> bool {
        self.config.mode == Mode::Permissive
    }

    fn seek_fragmented(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        if let Some(offset) = self.indexed_moof(track, decode_time) {
            return self.reposition(offset);
        }
        // A `sidx` that precedes the fragments has not necessarily been walked
        // past yet; scan for one before falling back to the trailing index.
        self.seek = Some(SeekRequest {
            track,
            decode_time,
            phase: SeekPhase::Scanning,
        });
        self.done = false;
        Ok(())
    }

    /// The `moof` offset an already-known index names for `track` at
    /// `decode_time`, when one of them covers the track.
    fn indexed_moof(&self, track: TrackId, decode_time: u64) -> Option<u64> {
        for index in &self.indexes {
            if index.index.reference_id != track.value() {
                continue;
            }
            if let Some(offset) = moof_in_sidx(index, decode_time) {
                return Some(offset);
            }
        }
        None
    }

    fn seek_phase_scanning(&self) -> bool {
        self.seek
            .is_some_and(|request| matches!(request.phase, SeekPhase::Scanning))
    }

    /// Fall back to the trailing `mfra`: needs the file length and a source
    /// that can read backwards.
    fn seek_via_mfra(&mut self, request: SeekRequest) -> Result<()> {
        let total = self.config.total_len.ok_or(Error::NoIndex {
            track: request.track,
        })?;
        if self.config.source != Source::Seekable || total < MFRO_BOX_LEN {
            return Err(Error::NoIndex {
                track: request.track,
            });
        }
        let range = ByteRange {
            offset: total - MFRO_BOX_LEN,
            len: MFRO_BOX_LEN,
        };
        self.pending = Some(Pending::MfraTrailer(range));
        self.seek = Some(SeekRequest {
            phase: SeekPhase::Trailer,
            ..request
        });
        Ok(())
    }

    /// Move the walk to `offset`, dropping whatever was pending.
    fn reposition(&mut self, offset: u64) -> Result<()> {
        if self.config.source == Source::ForwardOnly && offset < self.pos {
            return Err(Error::InvalidInput(
                "a backward seek needs a seekable source",
            ));
        }
        self.pos = offset;
        self.highest_requested = offset;
        self.pending = None;
        self.fragment = None;
        self.ready = None;
        self.last_fragment = None;
        self.deferred_warning = None;
        // Progressive samples of a hybrid file are not re-read; the seek
        // positions the fragment walk.
        self.cursors.clear();
        self.seek = None;
        self.done = false;
        Ok(())
    }

    fn accept_index(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let index = SegmentIndexBox::parse(bytes)?;
        if self.indexes.len() < MAX_REMEMBERED_INDEXES {
            self.indexes.push(SegmentIndex {
                index,
                anchor: range.end(),
            });
        }
        if self.seek_phase_scanning() {
            let Some(request) = self.seek else {
                return Ok(());
            };
            if let Some(offset) = self.indexed_moof(request.track, request.decode_time) {
                return self.reposition(offset);
            }
            // Another track's index (or none of them reaches the time): keep
            // scanning.
            self.seek = Some(request);
        }
        Ok(())
    }

    fn accept_mfra_trailer(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(request) = self.seek.take() else {
            return Err(Error::InvalidInput("mfra trailer with no seek pending"));
        };
        let total = self.config.total_len.ok_or(Error::NoIndex {
            track: request.track,
        })?;
        let Ok(offset_box) = MovieFragmentRandomAccessOffsetBox::parse(bytes) else {
            return Err(Error::NoIndex {
                track: request.track,
            });
        };
        let size = u64::from(offset_box.parent_size);
        if size < MFRO_BOX_LEN || size > total {
            return Err(Error::NoIndex {
                track: request.track,
            });
        }
        let range = ByteRange {
            offset: total - size,
            len: size,
        };
        self.request(range.offset, range.len)?;
        self.pending = Some(Pending::Mfra(range));
        self.seek = Some(SeekRequest {
            phase: SeekPhase::Mfra,
            ..request
        });
        Ok(())
    }

    fn accept_mfra(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(request) = self.seek.take() else {
            return Err(Error::InvalidInput("mfra with no seek pending"));
        };
        let mfra = MovieFragmentRandomAccessBox::parse(bytes)?;
        let offset = mfra
            .tfra
            .iter()
            .find(|tfra| tfra.track_id == request.track)
            .and_then(|tfra| moof_in_tfra(tfra, request.decode_time))
            .ok_or(Error::NoIndex {
                track: request.track,
            })?;
        self.reposition(offset)
    }
}

/// Whether a top-level box is carried for a rewrite. A trailing `mfra` is not:
/// its `tfra` entries address the source's fragments, so carrying it into a
/// rewritten file would leave a stale index (the writer does not rebuild one).
/// A `styp`/`emsg` belongs to the fragment run it precedes and is emitted by
/// the caller that writes that run.
fn carried_top_level(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    fourcc != b"mfra" && fourcc != b"styp" && fourcc != b"emsg"
}

/// Whether an opaque `stsd` holds an entry with a body, which means a known
/// four-CC failed to parse. A header-only entry is a caller's raw sample entry
/// and stays legal in both modes.
fn opaque_stsd_has_body(stsd: &[u8]) -> bool {
    let Some(body) = stsd.get(BOX_HEADER_MIN_SIZE..) else {
        return false;
    };
    if body.len() < FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() {
        return false;
    }
    let count = broadcast_common::wire::u32_at(body, FULLBOX_EXTRA_SIZE);
    let mut at = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
    for _ in 0..count {
        let Some(header) = body.get(at..at + BOX_HEADER_MIN_SIZE) else {
            return false;
        };
        let size = usize::try_from(broadcast_common::wire::u32_at(header, 0)).unwrap_or(0);
        if size > BOX_HEADER_MIN_SIZE {
            return true;
        }
        if size < BOX_HEADER_MIN_SIZE {
            return false;
        }
        at = at.saturating_add(size);
    }
    false
}

/// Whether a fragment parse failure is one the permissive mode tolerates.
///
/// A `LimitExceeded` is never tolerated: the caps bind both modes.
fn tolerable_fragment_error(error: &Error) -> bool {
    matches!(error, Error::BufferTooShort(_) | Error::InvalidInput(_))
}

/// The `moof` a `sidx` names for `decode_time` (§8.14.3).
///
/// Reference offsets start at the box's `first_offset` past the byte following
/// the `sidx`, and each later reference follows the previous one's
/// `referenced_size`. A nested index (`reference_type == 1`) is not a `moof`, so
/// it cannot be positioned on and yields `None`.
fn moof_in_sidx(index: &SegmentIndex, decode_time: u64) -> Option<u64> {
    let mut start = index.index.earliest_presentation_time;
    let mut offset = index.anchor.checked_add(index.index.first_offset)?;
    let mut last = None;
    for reference in &index.index.references {
        if reference.reference_type != 0 {
            return None;
        }
        let end = start.saturating_add(u64::from(reference.subsegment_duration));
        last = Some(offset);
        if decode_time < end {
            return Some(offset);
        }
        start = end;
        offset = offset.checked_add(u64::from(reference.referenced_size))?;
    }
    last
}

/// The `moof` a `tfra` names for `decode_time`: the last listed sync sample at
/// or before it, or the first when the time precedes every entry (§8.8.10).
fn moof_in_tfra(
    tfra: &crate::timing::TrackFragmentRandomAccessBox,
    decode_time: u64,
) -> Option<u64> {
    let mut chosen = None;
    for entry in &tfra.entries {
        if entry.time <= decode_time {
            chosen = Some(entry.moof_offset);
        } else {
            break;
        }
    }
    chosen.or_else(|| tfra.entries.first().map(|entry| entry.moof_offset))
}

impl Reader {
    /// The next step.
    ///
    /// Named `step`, not `next`: each step borrows the reader (the
    /// [`Step::Tracks`] arm hands out its own track table), which
    /// [`Iterator::next`]'s signature cannot express.
    pub fn step(&mut self) -> Result<Step<'_>> {
        if let Some(warning) = self.next_warning() {
            return Ok(Step::Warning(warning));
        }
        if let Some(pending) = self.pending {
            return Ok(Step::Need(pending.range()));
        }
        if self.done {
            return Ok(Step::Done);
        }
        if self.seek_phase_scanning() {
            return self.walk_step();
        }
        if self.ready.is_some()
            && let Some(step) = self.next_fragment_step()
        {
            return Ok(match step {
                FragmentStep::Fragment(info) => Step::Fragment(info),
                FragmentStep::Sample(sample) => Step::Sample(sample),
            });
        }
        if self.movie.is_some() {
            if !self.emitted_tracks {
                self.emitted_tracks = true;
                return Ok(Step::Tracks(&self.tracks));
            }
            if !self.emitted_metadata {
                self.emitted_metadata = true;
                if self.metadata.is_some() {
                    return self.metadata_step();
                }
            }
            if let Some(sample) = self.next_sample()? {
                return Ok(Step::Sample(sample));
            }
            if !self.fragmented && self.config.source == Source::ForwardOnly {
                // A forward-only caller has already streamed past the sample
                // data, so the boxes after it cannot be re-read; finish here
                // rather than demand a range behind the source's position.
                return self.finish_step();
            }
        }
        self.walk_step()
    }

    fn metadata_step(&self) -> Result<Step<'_>> {
        match self.metadata.as_ref() {
            Some(metadata) => Ok(Step::Metadata(metadata)),
            None => Err(Error::InvalidInput("metadata step without metadata")),
        }
    }

    /// Read the next top-level box header, or finish at the end of the input.
    fn walk_step(&mut self) -> Result<Step<'_>> {
        match self.next_header_range()? {
            Some(range) => {
                self.pending = Some(Pending::Header(range));
                Ok(Step::Need(range))
            }
            None => {
                if let Some(request) = self
                    .seek
                    .filter(|request| matches!(request.phase, SeekPhase::Scanning))
                {
                    self.seek_via_mfra(request)?;
                    if let Some(pending) = self.pending {
                        return Ok(Step::Need(pending.range()));
                    }
                }
                self.finish_fragment(Unresolved::Fail)?;
                if let Some(step) = self.next_fragment_step() {
                    return Ok(match step {
                        FragmentStep::Fragment(info) => Step::Fragment(info),
                        FragmentStep::Sample(sample) => Step::Sample(sample),
                    });
                }
                self.finish_step()
            }
        }
    }
}

impl Reader {
    fn next_header_range(&mut self) -> Result<Option<ByteRange>> {
        let len = match self.config.total_len {
            Some(total) if self.pos >= total => return Ok(None),
            Some(total) => (total - self.pos).min(MIN_BOX_HEADER_BYTES),
            None => MIN_BOX_HEADER_BYTES,
        };
        self.request(self.pos, len).map(Some)
    }

    fn request(&mut self, offset: u64, len: u64) -> Result<ByteRange> {
        if self.config.source == Source::ForwardOnly && offset < self.highest_requested {
            return Err(Error::MoovAfterMdatForwardOnly);
        }
        let range = ByteRange { offset, len };
        self.highest_requested = self.highest_requested.max(offset);
        Ok(range)
    }

    fn accept_header(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let offset = range.offset;
        if bytes.len() < BOX_HEADER_MIN_SIZE {
            if bytes.is_empty() && self.at_end(offset) {
                self.done = true;
                return Ok(());
            }
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: bytes.len(),
                    what: "top-level box header",
                },
            ));
        }
        let raw_size = broadcast_common::wire::u32_at(bytes, 0);
        let uuid = &bytes[4..8] == b"uuid";
        let largesize = raw_size == crate::box_types::SIZE_INDICATES_LARGESIZE;
        let required = MIN_BOX_HEADER_BYTES
            .saturating_add(if largesize {
                crate::box_types::LARGESIZE_SIZE as u64
            } else {
                0
            })
            .saturating_add(if uuid {
                crate::box_types::UUID_TYPE_SIZE as u64
            } else {
                0
            })
            .min(MAX_BOX_HEADER_BYTES);
        let supplied = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if supplied < required && range.len < required {
            let len = match self.config.total_len {
                Some(total) => total.saturating_sub(offset).min(required),
                None => required,
            };
            if len > range.len {
                self.pending = Some(Pending::Header(ByteRange { offset, len }));
                return Ok(());
            }
        }
        let header = BoxHeader::parse(bytes)?;
        let mut size = if header.size == 0 {
            let total = self.config.total_len.ok_or(Error::InvalidInput(
                "a size-0 box needs the total input length",
            ))?;
            total.checked_sub(offset).ok_or(Error::InvalidInput(
                "a size-0 box starts past the input end",
            ))?
        } else {
            header.size
        };
        let mut end = offset
            .checked_add(size)
            .ok_or(Error::InvalidInput("top-level box end overflow"))?;
        if let Some(total) = self.config.total_len
            && end > total
        {
            let truncatable = header.box_type.is(b"mdat") || header.box_type.is(b"moof");
            if self.tolerated() && truncatable {
                end = total;
                size = end.saturating_sub(offset);
                if header.box_type.is(b"mdat") {
                    if self.fragment.is_some() {
                        self.warn(Warning::TruncatedFragment { offset });
                    } else {
                        self.warn_truncated_mdat(offset);
                    }
                } else {
                    self.warn(Warning::TruncatedFragment { offset });
                    self.done = true;
                }
            } else {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: usize::try_from(size).unwrap_or(usize::MAX),
                        have: usize::try_from(total.saturating_sub(offset)).unwrap_or(usize::MAX),
                        what: "top-level box body",
                    },
                ));
            }
        }
        self.open_box = Some(ByteRange { offset, len: size });
        if header.box_type.is(b"ftyp") {
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "ftyp size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let range = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Ftyp(range));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"moov") {
            if self.config.source == Source::ForwardOnly && self.saw_mdat {
                return Err(Error::MoovAfterMdatForwardOnly);
            }
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "moov size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let range = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Moov(range));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"sidx") {
            if size > self.config.limits.max_moov_bytes() {
                return Err(Error::LimitExceeded {
                    what: "index size",
                    limit: self.config.limits.max_moov_bytes(),
                });
            }
            let index = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Index(index));
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"moof") {
            if let Some(request) = self
                .seek
                .filter(|request| matches!(request.phase, SeekPhase::Scanning))
            {
                // No `sidx` precedes the fragments: nothing is consumed here,
                // so the walk reads this header again once it has a position.
                self.pos = offset;
                return self.seek_via_mfra(request);
            }
            // The fragments a demuxer holds before their `mdat` share one
            // zero-width `trun` allowance; this reader holds one `moof` at a
            // time, so each gets a fresh one.
            self.finish_fragment(Unresolved::Fail)?;
            if size > self.config.limits.max_fragment_bytes() {
                return Err(Error::LimitExceeded {
                    what: "fragment size",
                    limit: self.config.limits.max_fragment_bytes(),
                });
            }
            let fragment = ByteRange { offset, len: size };
            self.request(offset, size)?;
            self.pending = Some(Pending::Moof(fragment));
            self.fragment = Some(FragmentState {
                offset,
                range: fragment,
                moof: None,
                mdats: Vec::new(),
            });
            self.pos = end;
            return Ok(());
        }
        if header.box_type.is(b"mdat") {
            self.saw_mdat = true;
            if let Some(fragment) = &mut self.fragment {
                let payload = offset.saturating_add(header.header_size() as u64);
                fragment.mdats.push(MdatRange::new(payload, end));
            }
        } else if self
            .fragment
            .as_ref()
            .is_some_and(|fragment| !fragment.mdats.is_empty() && !header.box_type.is(b"free"))
        {
            // A box other than `mdat`/`free` after a fragment's media ends that
            // fragment; nothing is consumed here, so the walk reads this header
            // again as the next step's first box.
            self.finish_fragment(Unresolved::Fail)?;
            self.pos = offset;
            return Ok(());
        }
        self.pos = end;
        if header.box_type.is(b"mdat") {
            // Every run of a fragment that resolves inside the `mdat` payloads
            // located so far is complete now: hand it out before the walk asks
            // for the header after its media, so a forward-only source has not
            // yet passed that payload.
            self.finish_fragment(Unresolved::Keep)?;
        } else if size <= self.config.limits.max_metadata_item_bytes()
            && carried_top_level(&header.box_type.0)
        {
            let range = ByteRange { offset, len: size };
            self.pending = Some(Pending::TopLevel(range));
        }
        Ok(())
    }

    /// Where a top-level box the writer does not model sits, relative to the
    /// boxes the writer emits.
    fn top_level_position(&self) -> BoxPosition {
        if self.saw_mdat {
            BoxPosition::AfterMdat
        } else if self.movie.is_some() {
            BoxPosition::AfterMoov
        } else {
            BoxPosition::AfterFtyp
        }
    }

    /// Keep one top-level box outside `ftyp`/`moov`/`mdat` so a rewrite can
    /// reproduce it in place.
    fn accept_top_level(&mut self, bytes: &[u8]) -> Result<()> {
        let (item, _) = parse_box(bytes)?;
        let carried = OpaqueBox {
            box_type: item.four_cc(),
            data: item.payload().to_vec(),
            to_end: item.to_end(),
            largesize: item.has_largesize(),
        };
        let position = self.top_level_position();
        self.top_level.push(TopLevelBox {
            position,
            data: carried,
        });
        Ok(())
    }

    fn accept_ftyp(&mut self, bytes: &[u8]) -> Result<()> {
        self.brand = Some(FileTypeBox::parse_box(bytes)?);
        Ok(())
    }

    fn accept_moof(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(fragment) = self.fragment.as_mut() else {
            return Err(Error::InvalidInput(
                "moof supplied with no fragment pending",
            ));
        };
        let offset = fragment.range.offset;
        let parsed = parse_box(bytes).and_then(|(header, _)| {
            let mut zero_width_budget = MAX_ZERO_WIDTH_TRUN_SAMPLES;
            MovieFragmentBox::parse_body_budgeted(header.body, &mut zero_width_budget)
        });
        match parsed {
            Ok(mut moof) => {
                apply_trex_defaults(&mut moof, &self.trex);
                fragment.moof = Some(moof);
                Ok(())
            }
            Err(err) if self.tolerated() && tolerable_fragment_error(&err) => {
                self.fragment = None;
                let truncated = self.config.total_len.is_some_and(|total| self.pos >= total);
                if !self.done {
                    self.warn(if truncated {
                        Warning::TruncatedFragment { offset }
                    } else {
                        Warning::MalformedFragment { offset }
                    });
                }
                if truncated {
                    self.done = true;
                }
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    fn accept_moov(&mut self, range: ByteRange, bytes: &[u8]) -> Result<()> {
        let mut owned = Vec::new();
        let source: &[u8] = if self.tolerated() {
            owned.extend_from_slice(bytes);
            sanitize_moov(&mut owned, range.offset, &mut self.warnings);
            &owned
        } else {
            bytes
        };
        let movie = MovieBox::parse(source)?;
        let mut tracks = Vec::with_capacity(movie.tracks.len());
        let mut cursors = Vec::new();
        let mut crypto = Vec::with_capacity(movie.tracks.len());
        let mut decode_times = Vec::with_capacity(movie.tracks.len());
        let mut sample_counts = Vec::with_capacity(movie.tracks.len());
        for (index, trak) in movie.tracks.iter().enumerate() {
            let raw = raw_stsd_entries(source, index, &self.config.limits, range.offset)?;
            self.check_opaque_stsd(trak, &raw)?;
            self.check_wave_esds(trak, &raw)?;
            let track_crypto = track_crypto(trak, &raw)?;
            let info = build_track_info(trak, raw, self.config.registry.as_ref())?;
            let mut declared_samples = 0u32;
            if let Some(stbl) = sample_table(trak) {
                let state = CursorState::new(
                    stbl,
                    info.track_id,
                    &self.config.limits,
                    self.config.total_len,
                )?;
                declared_samples = state.remaining();
                cursors.push(CursorSlot {
                    track_index: index,
                    state,
                });
            }
            crypto.push(track_crypto);
            decode_times.push(0);
            sample_counts.push(declared_samples);
            tracks.push(info);
        }
        self.fragmented = movie.mvex.is_some();
        self.trex = movie
            .mvex
            .as_ref()
            .map(|mvex| mvex.trex.clone())
            .unwrap_or_default();
        let mut movie_metadata = self.movie_metadata(&movie)?;
        if let Some(metadata) = movie_metadata.take() {
            self.metadata = Some(MovieMetadata {
                movie: metadata,
                tracks: Vec::new(),
            });
        }
        for trak in &movie.tracks {
            if let Some(metadata) = trak.metadata(&self.config.limits)? {
                self.metadata
                    .get_or_insert_with(MovieMetadata::default)
                    .tracks
                    .push((trak.tkhd.track_id, metadata));
            }
        }
        self.emitted_metadata = false;
        self.movie = Some(movie);
        self.tracks = tracks;
        self.cursors = cursors;
        self.crypto = crypto;
        self.decode_times = decode_times;
        self.sample_counts = sample_counts;
        self.emitted_tracks = false;
        Ok(())
    }

    fn movie_metadata(&self, movie: &MovieBox) -> Result<Option<crate::metadata::Metadata>> {
        movie.metadata(&self.config.limits)
    }

    /// Resolve the pending fragment's samples into [`Reader::ready`].
    ///
    /// A fragment with no `moof` yet (its body was never supplied because the
    /// input ended inside it) leaves [`Reader::ready`] empty. When a run does
    /// not resolve inside the `mdat` payloads located so far, `unresolved`
    /// decides whether that is the end of the fragment ([`Unresolved::Fail`])
    /// or a fragment still waiting for a later `mdat` ([`Unresolved::Keep`]).
    fn finish_fragment(&mut self, unresolved: Unresolved) -> Result<()> {
        let Some(state) = self.fragment.take() else {
            return Ok(());
        };
        let Some(moof) = state.moof.as_ref() else {
            return Ok(());
        };
        let at_end = self.config.total_len.is_some_and(|total| self.pos >= total);
        let partial = unresolved == Unresolved::Fail && at_end && self.tolerated();
        let resolved = match self.resolve_fragment(&state, moof, partial) {
            Ok(resolved) => resolved,
            Err(err) => {
                if unresolved == Unresolved::Keep {
                    self.fragment = Some(state);
                    return Ok(());
                }
                if self.tolerated() && tolerable_fragment_error(&err) {
                    let truncated = self.config.total_len.is_some_and(|total| self.pos >= total);
                    let already = self.warnings.iter().any(|warning| {
                        matches!(warning, Warning::TruncatedFragment { offset } if *offset == state.offset)
                    });
                    if !already {
                        self.warn(if truncated {
                            Warning::TruncatedFragment {
                                offset: state.offset,
                            }
                        } else {
                            Warning::MalformedFragment {
                                offset: state.offset,
                            }
                        });
                    }
                    if truncated {
                        self.done = true;
                    }
                    return Ok(());
                }
                return Err(err);
            }
        };
        if resolved.truncated {
            self.deferred_warning = Some(Warning::TruncatedFragment {
                offset: state.offset,
            });
        }
        for (index, decode_time, count) in resolved.cursors {
            self.decode_times[index] = decode_time;
            self.sample_counts[index] = count;
        }
        self.last_fragment = None;
        self.ready = Some(ReadyFragment {
            info: resolved.info,
            samples: resolved.samples,
            next: 0,
            emitted: false,
        });
        Ok(())
    }

    /// One fragment's samples, addressed through [`sample_ranges_in_run`]
    /// against the `mdat` payloads located so far, in the configured order.
    ///
    /// A run whose data a later `mdat` may hold is the resolver's own
    /// `InvalidInput`, which [`Reader::finish_fragment`] decides whether to
    /// treat as "not yet".
    fn resolve_fragment(
        &self,
        state: &FragmentState,
        moof: &MovieFragmentBox,
        partial: bool,
    ) -> Result<ResolvedFragment> {
        // The fragment's own media: a sample must live inside one of these
        // payloads, so non-overlapping samples cannot total more than they do.
        let budget = state.mdats.iter().fold(0u64, |total, range| {
            total.saturating_add(range.end().saturating_sub(range.start()))
        });
        let mut copied_bytes = 0u64;
        let mut rows: Vec<(usize, SampleRef)> = Vec::new();
        let mut truncated = false;
        let mut present: Vec<TrackId> = Vec::new();
        let mut cursors: Vec<(usize, u64, u32)> = Vec::new();
        for traf in &moof.traf {
            let track_id = traf.tfhd.track_id;
            let Some(index) = self
                .tracks
                .iter()
                .position(|info| info.track_id == track_id)
            else {
                continue;
            };
            present.push(track_id);
            let base = match &traf.tfdt {
                Some(tfdt) => tfdt.base_media_decode_time(),
                None => cursors
                    .iter()
                    .find(|slot| slot.0 == index)
                    .map_or(self.decode_times[index], |slot| slot.1),
            };
            let (ranges, cut) = if partial {
                sample_ranges_in_run_until(
                    state.offset,
                    moof,
                    track_id,
                    &state.mdats,
                    budget,
                    &mut copied_bytes,
                )?
            } else {
                (
                    sample_ranges_in_run(
                        state.offset,
                        moof,
                        track_id,
                        &state.mdats,
                        budget,
                        &mut copied_bytes,
                    )?,
                    false,
                )
            };
            if cut {
                truncated = true;
            }
            let auxes = fragment_aux(traf, self.crypto[index].as_ref(), ranges.len())?;
            let description_index = traf
                .tfhd
                .sample_description_index
                .or_else(|| {
                    self.trex
                        .iter()
                        .find(|trex| trex.track_id == track_id)
                        .map(|trex| trex.default_sample_description_index)
                })
                .unwrap_or(DEFAULT_DESCRIPTION_INDEX);
            let mut decode_time = base;
            let mut count = cursors
                .iter()
                .find(|slot| slot.0 == index)
                .map_or(self.sample_counts[index], |slot| slot.2);
            for (range, aux) in ranges.into_iter().zip(auxes) {
                let composition_offset = i32::try_from(range.composition_offset).map_err(|_| {
                    Error::InvalidInput("sample composition offset leaves the signed 32-bit range")
                })?;
                let sample = SampleRef {
                    track: track_id,
                    index: count,
                    decode_time,
                    composition_offset,
                    duration: range.duration,
                    sync: range.flags & SAMPLE_FLAG_IS_NON_SYNC == 0,
                    offset: range.start,
                    size: u32::try_from(range.end.saturating_sub(range.start))
                        .map_err(|_| Error::InvalidInput("sample size leaves the 32-bit range"))?,
                    description_index,
                    aux,
                };
                count = count.saturating_add(1);
                rows.push((index, sample));
                decode_time = decode_time.saturating_add(u64::from(range.duration));
            }
            match cursors.iter_mut().find(|slot| slot.0 == index) {
                Some(slot) => {
                    slot.1 = decode_time;
                    slot.2 = count;
                }
                None => cursors.push((index, decode_time, count)),
            }
            if truncated {
                break;
            }
        }
        let order = self.config.order;
        let tracks = &self.tracks;
        rows.sort_by(|(left_index, left), (right_index, right)| {
            compare_samples(
                order,
                left,
                tracks[*left_index].timescale,
                *left_index,
                right,
                tracks[*right_index].timescale,
                *right_index,
            )
        });
        Ok(ResolvedFragment {
            info: FragmentInfo {
                sequence_number: moof.mfhd.sequence_number,
                moof: state.range,
                tracks: present,
            },
            samples: rows.into_iter().map(|(_, sample)| sample).collect(),
            truncated,
            cursors,
        })
    }

    /// The next step of a resolved fragment, clearing it once its samples are
    /// exhausted.
    fn next_fragment_step(&mut self) -> Option<FragmentStep> {
        {
            let ready = self.ready.as_mut()?;
            if !ready.emitted {
                ready.emitted = true;
                return Some(FragmentStep::Fragment(ready.info.clone()));
            }
            if let Some(sample) = ready.samples.get(ready.next).cloned() {
                ready.next += 1;
                return Some(FragmentStep::Sample(sample));
            }
        }
        let ready = self.ready.take()?;
        self.last_fragment = Some((ready.info.moof.offset, ready.samples));
        None
    }

    fn at_end(&self, offset: u64) -> bool {
        self.config.total_len.is_none_or(|total| offset >= total)
    }

    fn next_sample(&mut self) -> Result<Option<SampleRef>> {
        let movie = self
            .movie
            .as_ref()
            .ok_or(Error::InvalidInput("samples before moov is parsed"))?;
        let order = self.config.order;
        let mut best: Option<(usize, usize, Timescale, SampleRef)> = None;
        for (slot_index, slot) in self.cursors.iter_mut().enumerate() {
            let info = self
                .tracks
                .get(slot.track_index)
                .ok_or(Error::InvalidInput("unknown track"))?;
            let trak = movie
                .tracks
                .get(slot.track_index)
                .ok_or(Error::UnexpectedBox { expected: "trak" })?;
            let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
            let Some(sample) = slot.state.peek(stbl)? else {
                continue;
            };
            let replace = match &best {
                None => true,
                Some((_, best_index, best_timescale, best_sample)) => {
                    compare_samples(
                        order,
                        &sample,
                        info.timescale,
                        slot.track_index,
                        best_sample,
                        *best_timescale,
                        *best_index,
                    ) == Ordering::Less
                }
            };
            if replace {
                best = Some((slot_index, slot.track_index, info.timescale, sample));
            }
        }
        let Some((slot_index, track_index, _, sample)) = best else {
            return Ok(None);
        };
        let slot = self
            .cursors
            .get_mut(slot_index)
            .ok_or(Error::InvalidInput("unknown cursor"))?;
        let trak = movie
            .tracks
            .get(track_index)
            .ok_or(Error::UnexpectedBox { expected: "trak" })?;
        let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
        if let Err(err) = slot.state.advance(stbl) {
            if self.tolerated() && matches!(err, Error::BufferTooShort(_)) {
                self.warn_truncated_mdat(sample.offset);
                return Ok(None);
            }
            return Err(err);
        }
        Ok(Some(sample))
    }
}

/// Total order over two heads: the configured primary key, ties broken by
/// `moov` track order.
fn compare_samples(
    order: SampleOrder,
    sample: &SampleRef,
    timescale: Timescale,
    track_index: usize,
    other: &SampleRef,
    other_timescale: Timescale,
    other_track_index: usize,
) -> Ordering {
    let primary = match order {
        SampleOrder::DecodeTime => {
            let left = u128::from(sample.decode_time) * u128::from(other_timescale.get());
            let right = u128::from(other.decode_time) * u128::from(timescale.get());
            left.cmp(&right)
        }
        SampleOrder::File => sample.offset.cmp(&other.offset),
    };
    primary.then_with(|| track_index.cmp(&other_track_index))
}

/// One past the sample's last byte.
fn extent(sample: &SampleRef) -> u64 {
    sample.offset.saturating_add(u64::from(sample.size))
}

/// One-based `stsd` entry a sample uses when neither its `tfhd` nor its
/// `trex` names one (ISO/IEC 14496-12:2026 §8.8.7.1).
const DEFAULT_DESCRIPTION_INDEX: u32 = 1;

/// Resolve a track's CENC defaults from the `sinf` of its sample entries.
///
/// A `seig` sample group would override them per run of samples (key rotation),
/// which this crate does not implement; a protected track that carries one is a
/// typed [`Error::UnsupportedFeature`] rather than samples decrypted with the
/// wrong `tenc`.
fn track_crypto(trak: &TrackBox, raw: &[RawEntry]) -> Result<Option<TrackCrypto>> {
    let mut tenc = None;
    for entry in raw {
        let Some(sinf) = sinf_of_sample_entry(&entry.body) else {
            continue;
        };
        let parsed = crate::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
        if let Some(found) = parsed
            .scheme_info
            .as_ref()
            .and_then(|info| info.tenc.clone())
        {
            tenc = Some(found);
            break;
        }
    }
    let Some(tenc) = tenc else {
        return Ok(None);
    };
    if stbl_has_seig_(trak)? {
        return Err(Error::UnsupportedFeature(
            "seig sample-group key rotation not implemented",
        ));
    }
    Ok(Some(TrackCrypto { tenc }))
}

/// Whether a track's `stbl` carries a `seig` sample group (ISO/IEC 23001-7
/// §12.5): key rotation this crate does not implement.
fn stbl_has_seig_(trak: &TrackBox) -> Result<bool> {
    let Some(stbl) = sample_table(trak) else {
        return Ok(false);
    };
    for child in &stbl.children {
        if let StblChild::Opaque(bytes) = child
            && let Ok(sgpd) = SampleGroupDescriptionBox::parse(bytes)
            && sgpd.grouping_type == GROUPING_TYPE_SEIG
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether one `traf` carries a `seig` sample group, the per-fragment form of
/// the same key rotation.
fn traf_has_seig(traf: &TrackFragmentBox) -> Result<bool> {
    for child in &traf.order {
        if let TrafChild::Opaque(opaque) = child
            && opaque.box_type == *b"sgpd"
            && let Ok(sgpd) = SampleGroupDescriptionBox::parse_body(&opaque.data)
            && sgpd.grouping_type == GROUPING_TYPE_SEIG
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One `traf`'s per-sample CENC auxiliary information, from its `senc` box
/// (ISO/IEC 23001-7 §12.3), one entry per sample of the track fragment.
///
/// The per-sample IV size is the track's `tenc.default_per_sample_iv_size`
/// (§12.2), the same rule the decrypt path uses. A constant-IV track has no
/// `senc` and yields no per-sample data, and an unprotected track yields none
/// either.
fn fragment_aux(
    traf: &TrackFragmentBox,
    crypto: Option<&TrackCrypto>,
    count: usize,
) -> Result<Vec<Option<SampleAux>>> {
    let Some(crypto) = crypto else {
        return Ok(core::iter::repeat_with(|| None).take(count).collect());
    };
    if traf_has_seig(traf)? {
        return Err(Error::UnsupportedFeature(
            "seig sample-group key rotation not implemented",
        ));
    }
    let senc = traf.order.iter().find_map(|child| match child {
        TrafChild::Opaque(opaque) if opaque.box_type == crate::cenc::SENC => Some(opaque),
        _ => None,
    });
    let Some(senc) = senc else {
        return Ok(core::iter::repeat_with(|| None).take(count).collect());
    };
    let data = &senc.data;
    if data.len() < FULLBOX_EXTRA_SIZE {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE,
                have: data.len(),
                what: "senc header",
            },
        ));
    }
    let version = data[0];
    let flags = broadcast_common::wire::u32_at(data, 0) & crate::box_types::FULLBOX_FLAGS_MASK;
    let parsed = SampleEncryptionBox::parse_body(
        &data[FULLBOX_EXTRA_SIZE..],
        version,
        flags,
        crypto.tenc.default_per_sample_iv_size,
    )?;
    if parsed.entries.len() < count {
        return Err(Error::InvalidInput(
            "senc has fewer entries than the fragment's samples",
        ));
    }
    Ok(parsed
        .entries
        .into_iter()
        .take(count)
        .map(|entry| {
            Some(SampleAux {
                initialization_vector: entry.initialization_vector,
                subsamples: entry.subsamples,
            })
        })
        .collect())
}

fn build_track_info(
    trak: &TrackBox,
    raw: Vec<RawEntry>,
    registry: Option<&crate::registry::BoxRegistry>,
) -> Result<TrackInfo> {
    let mdia: &MediaBox = trak
        .mdia
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
    let mdhd = mdia
        .mdhd
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "mdhd" })?;
    let hdlr = mdia
        .hdlr
        .as_ref()
        .ok_or(Error::UnexpectedBox { expected: "hdlr" })?;
    let stsd = mdia
        .minf
        .as_ref()
        .and_then(|minf| minf.stbl.as_ref())
        .and_then(sample_description);
    let mut entries = Vec::with_capacity(raw.len());
    for (index, raw) in raw.into_iter().enumerate() {
        let typed = stsd
            .and_then(|box_| box_.entries.get(index))
            .cloned()
            .and_then(|entry| match entry {
                SampleEntryVariant::Unknown(_) => None,
                known => Some(known),
            });
        let registered = match registry {
            Some(registry) => registry.parse(Some(STSD), &raw.body)?,
            None => None,
        };
        entries.push(SampleEntryInfo {
            fourcc: raw.fourcc,
            body: raw.body,
            typed,
            registered,
        });
    }
    let references = match trak.track_references()? {
        Some(reference) => reference.references,
        None => Vec::new(),
    };
    let edits = match trak.edts.as_ref().and_then(|edts| edts.elst.as_ref()) {
        Some(elst) => elst.entries.clone(),
        None => Vec::new(),
    };
    let tapt = trak
        .opaque
        .iter()
        .find(|b| b.box_type == crate::quicktime::aperture::TAPT)
        .and_then(|b| crate::quicktime::TrackApertureModeDimensionsBox::parse_body(&b.data).ok());
    Ok(TrackInfo {
        track_id: trak.tkhd.track_id,
        handler: hdlr.handler_type,
        timescale: mdhd.timescale,
        duration: mdhd.duration,
        header: trak.tkhd.clone(),
        references,
        edits,
        entries,
        tapt,
    })
}

fn raw_stsd_entries(
    moov: &[u8],
    track_index: usize,
    limits: &Limits,
    base_offset: u64,
) -> Result<Vec<RawEntry>> {
    let movie = parse_box(moov)?.0;
    let Some(trak) = nth_child(movie.body, b"trak", track_index, TRAK_DEPTH, limits)? else {
        return Ok(Vec::new());
    };
    let trak_body = parse_box(trak)?.0.body;
    let mdia = child(trak_body, b"mdia", MDIA_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "mdia" })?;
    let mdia_body = parse_box(mdia)?.0.body;
    let minf = child(mdia_body, b"minf", MINF_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "minf" })?;
    let minf_body = parse_box(minf)?.0.body;
    let stbl = child(minf_body, b"stbl", STBL_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "stbl" })?;
    let stbl_body = parse_box(stbl)?.0.body;
    let stsd = child(stbl_body, b"stsd", STSD_DEPTH, limits)?
        .ok_or(Error::UnexpectedBox { expected: "stsd" })?;
    let stsd_body = parse_box(stsd)?.0.body;
    let count = usize::try_from(broadcast_common::wire::be_u32(
        stsd_body,
        STSD_ENTRY_COUNT_OFFSET,
        "stsd entry_count",
    )?)
    .unwrap_or(usize::MAX);
    let entries_start = STSD_ENTRY_COUNT_OFFSET + core::mem::size_of::<u32>();
    let entries = stsd_body.get(entries_start..).unwrap_or(&[]);
    let base = moov.as_ptr() as usize;
    let mut out = Vec::new();
    for item in BoxIter::clamped(entries).take(count) {
        let Ok(item) = item else {
            break;
        };
        let offset = base_offset
            .saturating_add(u64::try_from(item.bytes.as_ptr() as usize - base).unwrap_or(u64::MAX));
        out.push(RawEntry {
            fourcc: item.four_cc(),
            body: item.bytes.to_vec(),
            offset,
        });
    }
    Ok(out)
}

fn child<'a>(
    body: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    depth: u32,
    limits: &Limits,
) -> Result<Option<&'a [u8]>> {
    check_depth(depth, limits)?;
    for item in BoxIter::new(body) {
        let item = item?;
        if item.header.box_type.is(fourcc) {
            return Ok(Some(item.bytes));
        }
    }
    Ok(None)
}

fn nth_child<'a>(
    body: &'a [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    index: usize,
    depth: u32,
    limits: &Limits,
) -> Result<Option<&'a [u8]>> {
    check_depth(depth, limits)?;
    let mut seen = 0usize;
    for item in BoxIter::new(body) {
        let item = item?;
        if item.header.box_type.is(fourcc) {
            if seen == index {
                return Ok(Some(item.bytes));
            }
            seen += 1;
        }
    }
    Ok(None)
}

fn check_depth(depth: u32, limits: &Limits) -> Result<()> {
    if depth > limits.max_depth() {
        return Err(Error::LimitExceeded {
            what: "box tree depth",
            limit: u64::from(limits.max_depth()),
        });
    }
    Ok(())
}

fn rescale_floor(value: u64, from: Timescale, to: Timescale) -> u64 {
    if from == to {
        return value;
    }
    let scaled = u128::from(value) * u128::from(to.get()) / u128::from(from.get());
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// Four-CC of the sample description box.
const STSD: [u8; BOX_TYPE_SIZE] = *b"stsd";

impl Reader {
    fn check_opaque_stsd(&mut self, trak: &TrackBox, raw: &[RawEntry]) -> Result<()> {
        let Some(stbl) = sample_table(trak) else {
            return Ok(());
        };
        let failed = stbl.children.iter().any(|child| match child {
            StblChild::Opaque(bytes)
                if bytes.len() >= BOX_HEADER_MIN_SIZE
                    && bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] == STSD =>
            {
                opaque_stsd_has_body(bytes)
            }
            _ => false,
        });
        if !failed {
            return Ok(());
        }
        if self.tolerated() {
            let (fourcc, offset) = raw.first().map_or(([0u8; BOX_TYPE_SIZE], 0), |entry| {
                (entry.fourcc, entry.offset)
            });
            self.warn(Warning::UnparseableSampleEntry { fourcc, offset });
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "a known sample-entry four-CC did not parse",
            ))
        }
    }

    fn check_wave_esds(&mut self, trak: &TrackBox, raw: &[RawEntry]) -> Result<()> {
        let Some(stsd) = trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.stbl.as_ref())
            .and_then(sample_description)
        else {
            return Ok(());
        };
        for (index, entry) in stsd.entries.iter().enumerate() {
            let SampleEntryVariant::Mp4a(mp4a) = entry else {
                continue;
            };
            let Some(wave) = mp4a.wave() else {
                continue;
            };
            if wave.esds.is_some() {
                continue;
            }
            let offset = raw.get(index).map_or(0, |entry| entry.offset);
            if self.tolerated() {
                self.warn(Warning::WaveWithoutEsds { offset });
            } else {
                return Err(Error::UnexpectedBox {
                    expected: "esds in an mp4a wave atom",
                });
            }
        }
        Ok(())
    }
}

/// Deepest container nesting the moov sanitizer descends.
const SANITIZE_MAX_DEPTH: u32 = 16;

/// Offset of the byte after a `meta` box's FullBox header.
const META_BODY_START: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
/// Body offset of the `mvhd` timescale in the version 0 form.
const MVHD_V0_TIMESCALE_BODY_OFFSET: usize = FULLBOX_EXTRA_SIZE + 8;
/// Body offset of the `mvhd` timescale in the version 1 form.
const MVHD_V1_TIMESCALE_BODY_OFFSET: usize = FULLBOX_EXTRA_SIZE + 16;

/// Container four-CCs the permissive `moov` sanitizer descends.
const SANITIZE_CONTAINERS: &[[u8; BOX_TYPE_SIZE]] = &[
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"edts", *b"udta", *b"mvex",
];

/// The `meta` box that carries a four-byte FullBox header before its children.
const META: [u8; BOX_TYPE_SIZE] = *b"meta";

/// Repair the faults a permissive `moov` walk tolerates.
fn sanitize_moov(data: &mut [u8], base: u64, warnings: &mut Vec<Warning>) {
    let body_start = crate::box_types::BoxHeader::parse(data)
        .map(|header| header.header_size())
        .unwrap_or(BOX_HEADER_MIN_SIZE);
    clamp_children(data, body_start, data.len(), base, warnings, 0);
    patch_zero_mvhd(data, body_start, base, warnings);
}

fn clamp_children(
    data: &mut [u8],
    start: usize,
    end: usize,
    base: u64,
    warnings: &mut Vec<Warning>,
    depth: u32,
) {
    if depth > SANITIZE_MAX_DEPTH {
        return;
    }
    let mut at = start;
    while at + BOX_HEADER_MIN_SIZE <= end {
        let size = broadcast_common::wire::u32_at(data, at);
        if size == 0 {
            return;
        }
        if size == 1 {
            return;
        }
        let mut size = usize::try_from(size).unwrap_or(usize::MAX);
        if size < BOX_HEADER_MIN_SIZE {
            return;
        }
        let mut fourcc = [0u8; BOX_TYPE_SIZE];
        fourcc.copy_from_slice(&data[at + BOX_TYPE_OFFSET..at + BOX_HEADER_MIN_SIZE]);
        if at.saturating_add(size) > end {
            let clamped = end.saturating_sub(at);
            let Ok(clamped32) = u32::try_from(clamped) else {
                return;
            };
            data[at..at + BOX_TYPE_OFFSET].copy_from_slice(&clamped32.to_be_bytes());
            warnings.push(Warning::ClampedBox {
                box_type: fourcc,
                offset: base.saturating_add(u64::try_from(at).unwrap_or(u64::MAX)),
            });
            size = clamped;
        }
        let child_body = if fourcc == META {
            at + META_BODY_START
        } else {
            at + BOX_HEADER_MIN_SIZE
        };
        if SANITIZE_CONTAINERS.contains(&fourcc) || fourcc == META {
            clamp_children(data, child_body, at + size, base, warnings, depth + 1);
        }
        at = at.saturating_add(size);
    }
}

fn patch_zero_mvhd(data: &mut [u8], start: usize, base: u64, warnings: &mut Vec<Warning>) {
    let mut found = None;
    let mut at = start;
    while at + BOX_HEADER_MIN_SIZE <= data.len() {
        let size = usize::try_from(broadcast_common::wire::u32_at(data, at)).unwrap_or(usize::MAX);
        if size < BOX_HEADER_MIN_SIZE {
            return;
        }
        if data[at + BOX_TYPE_OFFSET..at + BOX_HEADER_MIN_SIZE] == *b"mvhd" {
            found = Some((at, size));
            break;
        }
        let next = at.saturating_add(size);
        if next <= at || next > data.len() {
            return;
        }
        at = next;
    }
    let Some((at, size)) = found else {
        return;
    };
    let body = at + BOX_HEADER_MIN_SIZE;
    if body + FULLBOX_EXTRA_SIZE > data.len() || body + FULLBOX_EXTRA_SIZE > at + size {
        return;
    }
    let version = data[body];
    let timescale_at = if version == 1 {
        body + MVHD_V1_TIMESCALE_BODY_OFFSET
    } else {
        body + MVHD_V0_TIMESCALE_BODY_OFFSET
    };
    if timescale_at + core::mem::size_of::<u32>() > data.len() {
        return;
    }
    if broadcast_common::wire::u32_at(data, timescale_at) != 0 {
        return;
    }
    data[timescale_at..timescale_at + core::mem::size_of::<u32>()]
        .copy_from_slice(&1u32.to_be_bytes());
    warnings.push(Warning::ZeroMovieTimescale {
        offset: base.saturating_add(u64::try_from(at).unwrap_or(u64::MAX)),
    });
}
