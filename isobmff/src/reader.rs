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
//! defaults applied, the following `mdat` payload ranges learned from their
//! headers, and [`Step::Fragment`] followed by one [`Step::Sample`] per sample
//! is emitted before that `moof` is dropped. Fragment sample data is addressed
//! by [`crate::frag_offsets`] with those ranges, never with a slice of the
//! file, so no byte inside an `mdat` payload is ever requested.
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

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxHeader, BoxIter, FULLBOX_EXTRA_SIZE, parse_box,
};
use crate::cenc::{SampleEncryptionBox, TrackEncryptionBox, sinf_of_sample_entry};
use crate::error::{Error, Result};
use crate::frag_offsets::{MdatRange, sample_ranges_in_run};
use crate::init_segment::{
    MediaBox, MovieBox, SampleEntryVariant, StblChild, TrackBox, TrackExtendsBox,
};
use crate::limits::Limits;
use crate::movie_fragment::{
    MAX_ZERO_WIDTH_TRUN_SAMPLES, MovieFragmentBox, SAMPLE_FLAG_IS_NON_SYNC, TrackFragmentBox,
    TrafChild, apply_trex_defaults,
};
use crate::plan::{SamplePlan, sample_description, sample_table};
use crate::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox};
use crate::sample_table::{CursorState, SampleRef};
use crate::timing::EditListEntry;
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

/// One step of the reader state machine.
#[derive(Debug)]
#[non_exhaustive]
pub enum Step<'a> {
    /// Supply exactly this range with [`Reader::supply`].
    Need(ByteRange),
    /// Every track of the movie, in `moov` order. Emitted once.
    Tracks(&'a [TrackInfo]),
    /// A fragment's samples follow.
    Fragment(FragmentInfo),
    /// One sample descriptor, in the configured order.
    Sample(SampleRef),
    /// No more steps.
    Done,
}

/// The outstanding byte demand.
#[derive(Debug, Clone, Copy)]
enum Pending {
    Header(ByteRange),
    Moov(ByteRange),
    Moof(ByteRange),
}

impl Pending {
    fn range(&self) -> ByteRange {
        match self {
            Self::Header(range) | Self::Moov(range) | Self::Moof(range) => *range,
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

/// One owned step of a resolved fragment, so handing it out does not keep the
/// reader borrowed.
#[derive(Debug)]
enum FragmentStep {
    Fragment(FragmentInfo),
    Sample(SampleRef),
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
    tracks: Vec<TrackInfo>,
    cursors: Vec<CursorSlot>,
    emitted_tracks: bool,
    done: bool,
    fragmented: bool,
    trex: Vec<TrackExtendsBox>,
    crypto: Vec<Option<TrackCrypto>>,
    decode_times: Vec<u64>,
    sample_counts: Vec<u32>,
    fragment: Option<FragmentState>,
    ready: Option<ReadyFragment>,
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
            tracks: Vec::new(),
            cursors: Vec::new(),
            emitted_tracks: false,
            done: false,
            fragmented: false,
            trex: Vec::new(),
            crypto: Vec::new(),
            decode_times: Vec::new(),
            sample_counts: Vec::new(),
            fragment: None,
            ready: None,
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
            Pending::Moov(_) => self.accept_moov(bytes),
            Pending::Moof(_) => self.accept_moof(bytes),
        }
    }

    /// Bytes the reader has asked the caller to supply, under the
    /// `scan-counters` feature.
    #[cfg(feature = "scan-counters")]
    #[must_use]
    pub fn bytes_requested(&self) -> u64 {
        self.bytes_requested
    }

    /// Seek every track to its last sync sample at or before `decode_time`,
    /// expressed in `track`'s timescale.
    pub fn seek_to(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        let movie = self
            .movie
            .as_ref()
            .ok_or(Error::InvalidInput("seek before moov is parsed"))?;
        let named = self
            .tracks
            .iter()
            .position(|info| info.track_id == track)
            .ok_or(Error::InvalidInput("unknown track"))?;
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

    /// The progressive sample plan, once `moov` is parsed.
    #[must_use]
    pub fn plan(&self) -> Option<SamplePlan<'_>> {
        self.movie
            .as_ref()
            .map(|movie| SamplePlan::new(movie, &self.tracks, self.config.total_len))
    }
}

impl Reader {
    /// The next step.
    ///
    /// Named `step`, not `next`: each step borrows the reader (the
    /// [`Step::Tracks`] arm hands out its own track table), which
    /// [`Iterator::next`]'s signature cannot express.
    pub fn step(&mut self) -> Result<Step<'_>> {
        if let Some(pending) = self.pending {
            return Ok(Step::Need(pending.range()));
        }
        if self.done {
            return Ok(Step::Done);
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
            if let Some(sample) = self.next_sample()? {
                return Ok(Step::Sample(sample));
            }
            if !self.fragmented {
                self.done = true;
                return Ok(Step::Done);
            }
        }
        match self.next_header_range()? {
            Some(range) => {
                self.pending = Some(Pending::Header(range));
                Ok(Step::Need(range))
            }
            None => {
                self.finish_fragment()?;
                if let Some(step) = self.next_fragment_step() {
                    return Ok(match step {
                        FragmentStep::Fragment(info) => Step::Fragment(info),
                        FragmentStep::Sample(sample) => Step::Sample(sample),
                    });
                }
                self.done = true;
                Ok(Step::Done)
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
        let size = if header.size == 0 {
            let total = self.config.total_len.ok_or(Error::InvalidInput(
                "a size-0 box needs the total input length",
            ))?;
            total.checked_sub(offset).ok_or(Error::InvalidInput(
                "a size-0 box starts past the input end",
            ))?
        } else {
            header.size
        };
        let end = offset
            .checked_add(size)
            .ok_or(Error::InvalidInput("top-level box end overflow"))?;
        if let Some(total) = self.config.total_len
            && end > total
        {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: usize::try_from(size).unwrap_or(usize::MAX),
                    have: usize::try_from(total.saturating_sub(offset)).unwrap_or(usize::MAX),
                    what: "top-level box body",
                },
            ));
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
        if header.box_type.is(b"moof") {
            // The fragments a demuxer holds before their `mdat` share one
            // zero-width `trun` allowance; this reader holds one `moof` at a
            // time, so each gets a fresh one.
            self.finish_fragment()?;
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
            self.finish_fragment()?;
            self.pos = offset;
            return Ok(());
        }
        self.pos = end;
        Ok(())
    }

    fn accept_moof(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(fragment) = &mut self.fragment else {
            return Err(Error::InvalidInput(
                "moof supplied with no fragment pending",
            ));
        };
        let (header, _) = parse_box(bytes)?;
        let mut zero_width_budget = MAX_ZERO_WIDTH_TRUN_SAMPLES;
        let mut moof = MovieFragmentBox::parse_body_budgeted(header.body, &mut zero_width_budget)?;
        apply_trex_defaults(&mut moof, &self.trex);
        fragment.moof = Some(moof);
        Ok(())
    }

    fn accept_moov(&mut self, bytes: &[u8]) -> Result<()> {
        let movie = MovieBox::parse(bytes)?;
        let mut tracks = Vec::with_capacity(movie.tracks.len());
        let mut cursors = Vec::new();
        let mut crypto = Vec::with_capacity(movie.tracks.len());
        let mut decode_times = Vec::with_capacity(movie.tracks.len());
        let mut sample_counts = Vec::with_capacity(movie.tracks.len());
        for (index, trak) in movie.tracks.iter().enumerate() {
            let raw = raw_stsd_entries(bytes, index, &self.config.limits)?;
            let track_crypto = track_crypto(trak, &raw)?;
            let info = build_track_info(trak, raw)?;
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
        self.movie = Some(movie);
        self.tracks = tracks;
        self.cursors = cursors;
        self.crypto = crypto;
        self.decode_times = decode_times;
        self.sample_counts = sample_counts;
        self.emitted_tracks = false;
        Ok(())
    }

    /// Resolve the pending fragment's samples into [`Reader::ready`].
    ///
    /// Nothing is resolved until the `moof` box has been supplied; a fragment
    /// with no `moof` yet (its body was never supplied because the input ended
    /// inside it) leaves [`Reader::ready`] empty, and a truncated fragment's
    /// samples fail the resolver's own bounds.
    fn finish_fragment(&mut self) -> Result<()> {
        let Some(state) = self.fragment.take() else {
            return Ok(());
        };
        let Some(moof) = state.moof else {
            return Ok(());
        };
        // The fragment's own media: a sample must live inside one of these
        // payloads, so non-overlapping samples cannot total more than they do.
        let budget = state.mdats.iter().fold(0u64, |total, range| {
            total.saturating_add(range.end().saturating_sub(range.start()))
        });
        let mut copied_bytes = 0u64;
        let mut rows: Vec<(usize, SampleRef)> = Vec::new();
        let mut present: Vec<TrackId> = Vec::new();
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
                None => self.decode_times[index],
            };
            let ranges = sample_ranges_in_run(
                state.offset,
                &moof,
                track_id,
                &state.mdats,
                budget,
                &mut copied_bytes,
            )?;
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
            for (range, aux) in ranges.into_iter().zip(auxes) {
                let composition_offset = i32::try_from(range.composition_offset).map_err(|_| {
                    Error::InvalidInput("sample composition offset leaves the signed 32-bit range")
                })?;
                let sample = SampleRef {
                    track: track_id,
                    index: self.sample_counts[index],
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
                self.sample_counts[index] = self.sample_counts[index].saturating_add(1);
                rows.push((index, sample));
                decode_time = decode_time.saturating_add(u64::from(range.duration));
            }
            self.decode_times[index] = decode_time;
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
        self.ready = Some(ReadyFragment {
            info: FragmentInfo {
                sequence_number: moof.mfhd.sequence_number,
                moof: state.range,
                tracks: present,
            },
            samples: rows.into_iter().map(|(_, sample)| sample).collect(),
            next: 0,
            emitted: false,
        });
        Ok(())
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
        self.ready = None;
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
        slot.state.advance(stbl)?;
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
    let flags = u32::from_be_bytes([0, data[1], data[2], data[3]]);
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

fn build_track_info(trak: &TrackBox, raw: Vec<RawEntry>) -> Result<TrackInfo> {
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
        entries.push(SampleEntryInfo {
            fourcc: raw.fourcc,
            body: raw.body,
            typed,
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
    Ok(TrackInfo {
        track_id: trak.tkhd.track_id,
        handler: hdlr.handler_type,
        timescale: mdhd.timescale,
        duration: mdhd.duration,
        header: trak.tkhd.clone(),
        references,
        edits,
        entries,
    })
}

fn raw_stsd_entries(moov: &[u8], track_index: usize, limits: &Limits) -> Result<Vec<RawEntry>> {
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
    let mut out = Vec::new();
    for item in BoxIter::clamped(entries).take(count) {
        let Ok(item) = item else {
            break;
        };
        out.push(RawEntry {
            fourcc: item.four_cc(),
            body: item.bytes.to_vec(),
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
