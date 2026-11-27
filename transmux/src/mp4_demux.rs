//! The streaming MP4 demux stage over [`isobmff::reader::Reader`].
//!
//! [`Mp4Demux`] is the one MP4 demuxer: it answers the reader's byte demands
//! through [`Stage::demand`] and emits the crate's existing [`DemuxEvent`]
//! vocabulary, so a segment consumer keeps one stage per representation and a
//! file consumer reads through bounded byte ranges. It never holds a whole
//! file: a seekable input delivers exactly the ranges the reader asks for, and
//! a sequential input holds at most one bounded unit (a `moov`, or a `moof`
//! plus the `mdat` payload its runs address).
//!
//! The reader is always created in [`Mode::Permissive`]: a fault it tolerates
//! becomes [`DemuxEvent::InputDegraded`], never a silent skip. A fault it
//! cannot tolerate is the stage's error, after which the stage is poisoned.
//! A sequential consumer marks each media segment with [`Mp4Input::SegmentEnd`]:
//! the stage drops any partially read top-level box, keeps the parsed `moov`
//! and its track state, and resumes at the next segment's first byte.
//!
//! Sample timing and conditioning are exactly the whole-input demuxers':
//! `dts` is the reader's absolute decode time, `pts = dts + composition
//! offset`, and the shared helpers in [`crate::media`] recover each track's
//! codec config and per-access-unit display state.
//!
//! See the streaming-first design §4.2 and §4.4 and ISO/IEC 14496-12:2026 §4.2,
//! §8.2-§8.7 (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use broadcast_common::stage::{Demand, Stage, Timestamp};
use broadcast_core::timescale::Timescale;
use bytes::{Buf, Bytes, BytesMut};
use core::num::NonZeroUsize;
use isobmff::Limits;
use isobmff::reader::{
    ByteRange, FragmentInfo, Mode, Reader, ReaderConfig, SampleOrder, Source, Step, TrackInfo,
    Warning,
};
use isobmff::sample_table::SampleRef;

use crate::error::{Error, Result};
use crate::ir::{
    AbandonReason, CodecConfig, DemuxEvent, EventProvenance, InputDegradation, Sample, SampleFlags,
    SampleProtection, SampleTiming, TrackSpec,
};
use crate::media::{
    InbandDisplayTagger, nal_length_size, normalise_nal_lengths, normalise_spec_nal_length,
    refine_legacy_config, track_entry_inband_and_extras, track_spec_from_track_info,
};

/// The fragment ordinal at which unresolved tracks are declared resolved.
const FRAGMENTS_BEFORE_RESOLVE: u32 = 2;

/// Bytes in a box header's 32-bit `size` field (ISO/IEC 14496-12:2026 §4.2).
const BOX_SIZE_FIELD_SIZE: usize = 4;

/// Capacity a sequential stage retains at a segment end. Nothing: the buffer a
/// finished segment grew must not become the next segment's floor.
const SEGMENT_END_RETAINED_CAPACITY: usize = 0;

/// Which way the stage reaches its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceMode {
    /// Arbitrary byte ranges are available (a file).
    Seekable,
    /// The input only moves forward (a pipe or socket).
    Sequential,
}

/// One unit of input handed to [`Mp4Demux`].
#[non_exhaustive]
#[derive(Debug)]
pub enum Mp4Input<'a> {
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
    /// A sequential media segment ended here; the next bytes start a new one.
    SegmentEnd,
    /// No more input will arrive.
    Eof,
}

/// Configuration for [`Mp4Demux`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Mp4DemuxConfig {
    limits: Limits,
    order: SampleOrder,
    read_window: Option<NonZeroUsize>,
    total_len: Option<u64>,
}

impl Default for Mp4DemuxConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            order: SampleOrder::DecodeTime,
            read_window: None,
            total_len: None,
        }
    }
}

impl Mp4DemuxConfig {
    /// Set the hostile-input caps.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Set the sample order (seekable mode only; sequential mode always reads
    /// file order).
    #[must_use]
    pub fn with_order(mut self, order: SampleOrder) -> Self {
        self.order = order;
        self
    }

    /// Set the largest byte range one sample request may cover. `None` (the
    /// default) requests exactly one sample at a time; the library never picks
    /// a window itself.
    #[must_use]
    pub fn with_read_window(mut self, read_window: Option<NonZeroUsize>) -> Self {
        self.read_window = read_window;
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

    /// The sample order.
    #[must_use]
    pub fn order(&self) -> SampleOrder {
        self.order
    }

    /// The largest byte range one sample request may cover.
    #[must_use]
    pub fn read_window(&self) -> Option<NonZeroUsize> {
        self.read_window
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
    Tracks(Vec<TrackInfo>),
    Fragment(FragmentInfo),
    Sample(SampleRef),
    Warning(Warning),
    Done,
}

/// One mapped track's conditioning state.
struct Mapped {
    spec: TrackSpec,
    length_size: Option<usize>,
    in_band: bool,
    extra: Vec<isobmff::sample_entries::OpaqueBox>,
    tagger: Option<InbandDisplayTagger>,
    live: bool,
}

/// One track of the movie, mapped or abandoned.
struct TrackState {
    id: isobmff::TrackId,
    mapped: Option<Mapped>,
    abandoned: bool,
}

impl TrackState {
    fn is_live(&self) -> bool {
        self.mapped.as_ref().is_some_and(|mapped| mapped.live)
    }

    fn is_resolved(&self) -> bool {
        self.abandoned || self.is_live()
    }
}

/// The streaming MP4 demux stage.
pub struct Mp4Demux {
    config: Mp4DemuxConfig,
    source: SourceMode,
    reader: Reader,
    events: VecDeque<DemuxEvent>,
    tracks: Vec<TrackState>,
    saw_tracks: bool,
    resolved: bool,
    fragments: u32,
    last_structure: u64,
    staged: Option<OwnedStep>,
    structure: Option<ByteRange>,
    batch: Vec<SampleRef>,
    batch_range: Option<ByteRange>,
    buf: BytesMut,
    buf_start: u64,
    eof: bool,
    done: bool,
    finished: bool,
    flushed: bool,
    poisoned: Option<Error>,
}

impl Mp4Demux {
    /// A stage over a source that can serve any byte range (a file).
    #[must_use]
    pub fn seekable(config: Mp4DemuxConfig) -> Self {
        let reader_config = ReaderConfig::default()
            .with_total_len(config.total_len)
            .with_order(config.order)
            .with_source(Source::Seekable)
            .with_mode(Mode::Permissive)
            .with_limits(config.limits);
        Self::create(config, SourceMode::Seekable, reader_config)
    }

    /// A stage over a source that only moves forward (a pipe or socket).
    #[must_use]
    pub fn sequential(config: Mp4DemuxConfig) -> Self {
        let reader_config = ReaderConfig::default()
            .with_total_len(config.total_len)
            .with_order(SampleOrder::File)
            .with_source(Source::ForwardOnly)
            .with_mode(Mode::Permissive)
            .with_limits(config.limits);
        Self::create(config, SourceMode::Sequential, reader_config)
    }

    fn create(config: Mp4DemuxConfig, source: SourceMode, reader_config: ReaderConfig) -> Self {
        let mut demux = Self {
            config,
            source,
            reader: Reader::new(reader_config),
            events: VecDeque::new(),
            tracks: Vec::new(),
            saw_tracks: false,
            resolved: false,
            fragments: 0,
            last_structure: 0,
            staged: None,
            structure: None,
            batch: Vec::new(),
            batch_range: None,
            buf: BytesMut::new(),
            buf_start: 0,
            eof: false,
            done: false,
            finished: false,
            flushed: false,
            poisoned: None,
        };
        if let Err(err) = demux.pump() {
            demux.poisoned = Some(err);
        }
        demux
    }

    /// The movie timescale, once the `moov` is parsed.
    #[must_use]
    pub fn movie_timescale(&self) -> Option<Timescale> {
        self.reader.movie_timescale()
    }

    /// Whether the movie is fragmented (`moov` carries `mvex`), once the track
    /// table is known.
    #[must_use]
    pub fn is_fragmented(&self) -> Option<bool> {
        self.reader.is_fragmented()
    }

    /// Every mapped track's spec, as the `moov` announced it, available as
    /// soon as that `moov` has been parsed (an init alone, no `finish`).
    ///
    /// The NAL length prefix is already canonical here. A legacy codec's
    /// provisional config is the reader's; the first sample may refine it, and
    /// [`DemuxEvent::TrackAdded`] then carries the refined spec.
    pub fn init_tracks(&self) -> impl Iterator<Item = &TrackSpec> + '_ {
        self.tracks
            .iter()
            .filter_map(|state| state.mapped.as_ref().map(|mapped| &mapped.spec))
    }

    /// Bytes the sequential buffer currently holds, and the capacity it keeps
    /// allocated for more.
    ///
    /// A consumer that sizes its own reads can assert its memory policy
    /// against this. After a segment end the stage retains no capacity, never
    /// the finished segment's.
    #[must_use]
    pub fn buffered(&self) -> (usize, usize) {
        (self.buf.len(), self.buf.capacity())
    }

    /// Declare that the bytes fed so far end one sequential media segment and
    /// keep the `moov` and its track state for the next.
    ///
    /// The reader drops any top-level box whose declared end runs past the
    /// segment: in its permissive mode the cut fragment is reported as
    /// [`InputDegradation::MalformedUnit`], the kind the whole-input demuxers
    /// use for the same damage. The buffered partial bytes are dropped, so a
    /// damaged segment cannot affect the next one.
    fn segment_end(&mut self) -> Result<()> {
        let unit_end = self.buf_end();
        if let Some(OwnedStep::Warning(warning)) = self.staged.take() {
            self.push_warning(warning);
        }
        self.staged = None;
        self.structure = None;
        self.batch_range = None;
        self.batch.clear();
        let result = self.reader.end_of_unit(unit_end).map_err(Error::from);
        let pumped = result.and_then(|()| self.pump());
        self.buf = BytesMut::with_capacity(SEGMENT_END_RETAINED_CAPACITY);
        self.buf_start = unit_end;
        self.done = false;
        pumped
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
            if self.done {
                self.finish_flush();
                return Ok(());
            }
            if let Some(range) = self.batch_range.take() {
                match self.source {
                    SourceMode::Sequential => {
                        if self.emit_batch_from_buffer(range)? {
                            continue;
                        }
                        self.batch_range = Some(range);
                        return Ok(());
                    }
                    SourceMode::Seekable => {
                        if self.eof {
                            self.push_truncated(range.offset);
                            self.batch.clear();
                            continue;
                        }
                        self.batch_range = Some(range);
                        return Ok(());
                    }
                }
            }
            if let Some(range) = self.structure.take() {
                match self.source {
                    SourceMode::Sequential => {
                        if self.supply_structure_from_buffer(range)? {
                            continue;
                        }
                        self.structure = Some(range);
                        return Ok(());
                    }
                    SourceMode::Seekable => {
                        if self.eof {
                            if self.supply_structure_at_eof(range)? {
                                continue;
                            }
                            return Ok(());
                        }
                        self.structure = Some(range);
                        return Ok(());
                    }
                }
            }
            let Some(step) = self.next_owned_step()? else {
                continue;
            };
            if !self.batch.is_empty() && !matches!(step, OwnedStep::Sample(_)) {
                self.staged = Some(step);
                self.seal_batch();
                continue;
            }
            match step {
                OwnedStep::Need(range) => {
                    self.structure = Some(range);
                }
                OwnedStep::Tracks(infos) => self.on_tracks(infos)?,
                OwnedStep::Fragment(info) => self.on_fragment(info),
                OwnedStep::Sample(sref) => {
                    if !self.on_sample(sref)? {
                        return Ok(());
                    }
                }
                OwnedStep::Warning(warning) => self.push_warning(warning),
                OwnedStep::Done => self.done = true,
            }
        }
    }
}

impl Mp4Demux {
    fn next_owned_step(&mut self) -> Result<Option<OwnedStep>> {
        if let Some(step) = self.staged.take() {
            return Ok(Some(step));
        }
        let step = match self.reader.step() {
            Ok(step) => step,
            Err(isobmff::Error::SampleDataBudgetExceeded { .. }) => {
                self.push_malformed(self.last_structure);
                self.done = true;
                return Ok(None);
            }
            Err(err) => return Err(err.into()),
        };
        Ok(match step {
            Step::Need(range) => Some(OwnedStep::Need(range)),
            Step::Tracks(infos) => Some(OwnedStep::Tracks(infos.to_vec())),
            Step::Metadata(_) => None,
            Step::Fragment(info) => Some(OwnedStep::Fragment(info)),
            Step::Sample(sref) => Some(OwnedStep::Sample(sref)),
            Step::Warning(warning) => Some(OwnedStep::Warning(warning)),
            Step::Done => Some(OwnedStep::Done),
            _ => None,
        })
    }

    fn on_tracks(&mut self, infos: Vec<TrackInfo>) -> Result<()> {
        if self.saw_tracks {
            return Err(Error::InvalidInput(
                "MP4 input carries a second moov; start a new Mp4Demux stage",
            ));
        }
        self.saw_tracks = true;
        self.tracks.clear();
        for info in infos {
            match track_spec_from_track_info(&info) {
                Ok((mut spec, finding)) => {
                    if let Some(finding) = finding {
                        self.events.push_back(DemuxEvent::GaplessSignalConflict {
                            track_id: info.track_id.into(),
                            used: finding.used,
                            ignored: finding.ignored,
                        });
                    }
                    let (in_band, extra) = info
                        .entries
                        .first()
                        .and_then(|entry| entry.typed.as_ref())
                        .map_or((false, Vec::new()), track_entry_inband_and_extras);
                    let length_size = nal_length_size(&spec.config);
                    normalise_spec_nal_length(&mut spec);
                    self.tracks.push(TrackState {
                        id: info.track_id,
                        mapped: Some(Mapped {
                            length_size,
                            spec,
                            in_band,
                            extra,
                            tagger: None,
                            live: false,
                        }),
                        abandoned: false,
                    });
                }
                Err(err) => {
                    self.events.push_back(DemuxEvent::TrackAbandoned {
                        track_id: Some(info.track_id.into()),
                        reason: abandon_reason(&err, &info),
                        provenance: EventProvenance::default(),
                    });
                    self.tracks.push(TrackState {
                        id: info.track_id,
                        mapped: None,
                        abandoned: true,
                    });
                }
            }
        }
        self.maybe_resolve();
        Ok(())
    }

    fn on_fragment(&mut self, _info: FragmentInfo) {
        self.fragments = self.fragments.saturating_add(1);
        if self.fragments == FRAGMENTS_BEFORE_RESOLVE {
            self.emit_resolved();
        }
    }

    fn maybe_resolve(&mut self) {
        if self.tracks.iter().all(TrackState::is_resolved) {
            self.emit_resolved();
        }
    }

    fn emit_resolved(&mut self) {
        if self.resolved {
            return;
        }
        self.resolved = true;
        self.events
            .push_back(DemuxEvent::TracksResolved { generation: 0 });
    }

    fn finish_flush(&mut self) {
        if self.flushed {
            return;
        }
        self.flushed = true;
        let specs: Vec<TrackSpec> = self
            .tracks
            .iter_mut()
            .filter_map(|state| {
                let mapped = state.mapped.as_mut()?;
                if mapped.live {
                    return None;
                }
                mapped.live = true;
                Some(mapped.spec.clone())
            })
            .collect();
        for spec in specs {
            self.events.push_back(DemuxEvent::TrackAdded(spec));
        }
        self.emit_resolved();
    }

    fn push_warning(&mut self, warning: Warning) {
        let position = warning.position();
        let kind = match warning {
            Warning::TruncatedMdat { .. } => InputDegradation::TruncatedUnit { position },
            _ => InputDegradation::MalformedUnit { position },
        };
        self.events.push_back(DemuxEvent::InputDegraded {
            track: None,
            kind,
            provenance: EventProvenance::default(),
        });
    }

    fn push_malformed(&mut self, position: u64) {
        self.events.push_back(DemuxEvent::InputDegraded {
            track: None,
            kind: InputDegradation::MalformedUnit { position },
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

fn abandon_reason(err: &Error, info: &TrackInfo) -> AbandonReason {
    match err {
        Error::UnsupportedSampleEntry { fourcc }
            if fourcc.len() == isobmff::box_types::BOX_TYPE_SIZE =>
        {
            let mut bytes = [0u8; isobmff::box_types::BOX_TYPE_SIZE];
            for (slot, byte) in bytes.iter_mut().zip(fourcc.as_bytes()) {
                *slot = *byte;
            }
            let detail = info
                .entries
                .iter()
                .find(|entry| entry.fourcc == bytes)
                .and_then(|entry| entry.error.clone());
            AbandonReason::UnsupportedSampleEntry {
                fourcc: bytes,
                detail,
            }
        }
        Error::UnsupportedSampleEntry { .. } => AbandonReason::UnsupportedCodec,
        Error::UnexpectedBox { expected } => AbandonReason::MalformedTrack { expected },
        _ => AbandonReason::ConfigUnrecoverable,
    }
}

impl Mp4Demux {
    fn on_sample(&mut self, sref: SampleRef) -> Result<bool> {
        if sref.size == 0 {
            if self.batch.is_empty() {
                self.emit_sample(sref, Bytes::new())?;
                return Ok(true);
            }
            self.staged = Some(OwnedStep::Sample(sref));
            self.seal_batch();
            return Ok(false);
        }
        match self.source {
            SourceMode::Seekable => {
                if self.batch.is_empty() {
                    self.batch.push(sref);
                    if self.config.read_window.is_none() {
                        self.seal_batch();
                        return Ok(false);
                    }
                    return Ok(true);
                }
                if self.can_merge(&sref) {
                    self.batch.push(sref);
                    if self.batch.len() >= self.batch_limit() {
                        self.seal_batch();
                        return Ok(false);
                    }
                    return Ok(true);
                }
                self.staged = Some(OwnedStep::Sample(sref));
                self.seal_batch();
                Ok(false)
            }
            SourceMode::Sequential => {
                if self.batch.is_empty() {
                    self.batch.push(sref);
                    return Ok(true);
                }
                let (start, end) = self
                    .batch_bounds()
                    .unwrap_or((sref.offset, sample_end(&sref)));
                let union_start = start.min(sref.offset);
                let union_end = end.max(sample_end(&sref));
                let cap = self.config.limits.max_need_bytes();
                if union_end.saturating_sub(union_start) > cap {
                    self.staged = Some(OwnedStep::Sample(sref));
                    self.seal_batch();
                    return Ok(false);
                }
                self.batch.push(sref);
                Ok(true)
            }
        }
    }

    fn can_merge(&self, sref: &SampleRef) -> bool {
        let Some((start, end)) = self.batch_bounds() else {
            return true;
        };
        let Some(window) = self.config.read_window else {
            return false;
        };
        let window = u64::try_from(window.get()).unwrap_or(u64::MAX);
        if sref.offset < start {
            return false;
        }
        if sref.offset.saturating_sub(end) > window {
            return false;
        }
        end.max(sample_end(sref)).saturating_sub(start) <= window
    }

    fn batch_bounds(&self) -> Option<(u64, u64)> {
        let first = self.batch.first()?;
        let mut start = first.offset;
        let mut end = sample_end(first);
        for sref in self.batch.iter().skip(1) {
            start = start.min(sref.offset);
            end = end.max(sample_end(sref));
        }
        Some((start, end))
    }

    fn seal_batch(&mut self) {
        let Some((start, end)) = self.batch_bounds() else {
            return;
        };
        self.batch_range = Some(ByteRange {
            offset: start,
            len: end.saturating_sub(start),
        });
    }

    fn batch_limit(&self) -> usize {
        usize::try_from(self.config.limits.max_samples_per_track()).unwrap_or(usize::MAX)
    }

    fn emit_batch_from_buffer(&mut self, range: ByteRange) -> Result<bool> {
        let available_end = self.buf_end();
        if range.offset > available_end {
            if self.eof {
                self.push_truncated(range.offset);
                self.batch.clear();
                self.done = true;
                return Ok(true);
            }
            return Ok(false);
        }
        if range.offset > self.buf_start {
            self.discard_to(range.offset);
        }
        if range.offset < self.buf_start {
            return Err(Error::InvalidInput(
                "MP4 sample range lies behind the sequential buffer",
            ));
        }
        if self.buf_end() < range.end() {
            if !self.eof {
                return Ok(false);
            }
            self.truncate_batch();
            return Ok(true);
        }
        let samples = core::mem::take(&mut self.batch);
        for sref in samples {
            let start = usize::try_from(sref.offset.saturating_sub(self.buf_start)).unwrap_or(0);
            let size = usize::try_from(sref.size).unwrap_or(0);
            let end = start.saturating_add(size);
            if end > self.buf.len() {
                self.push_truncated(sref.offset);
                break;
            }
            let data = Bytes::copy_from_slice(&self.buf[start..end]);
            self.emit_sample(sref, data)?;
        }
        self.batch_range = None;
        self.discard_to(range.end());
        Ok(true)
    }

    fn truncate_batch(&mut self) {
        let samples = core::mem::take(&mut self.batch);
        for sref in samples {
            if sample_end(&sref) <= self.buf_end() {
                let start =
                    usize::try_from(sref.offset.saturating_sub(self.buf_start)).unwrap_or(0);
                let size = usize::try_from(sref.size).unwrap_or(0);
                let data = Bytes::copy_from_slice(&self.buf[start..start.saturating_add(size)]);
                if let Err(err) = self.emit_sample(sref, data) {
                    self.poisoned = Some(err);
                    break;
                }
            } else {
                self.push_truncated(sref.offset);
                break;
            }
        }
        self.batch_range = None;
    }

    fn emit_sample(&mut self, sref: SampleRef, data: Bytes) -> Result<()> {
        let Some(index) = self.track_position(sref.track) else {
            return Ok(());
        };
        let dts = i64::try_from(sref.decode_time)
            .map_err(|_| Error::InvalidInput("decode time exceeds i64"))?;
        let pts = isobmff::frag_offsets::add_offset(dts, i64::from(sref.composition_offset))?;
        let mut sample = Sample {
            data,
            timing: Some(SampleTiming::timed_with_duration(dts, pts, sref.duration)),
            flags: SampleFlags::new(sref.sync),
            provenance: None,
            display: None,
            protection: sref
                .aux
                .as_ref()
                .map(|aux| Arc::new(SampleProtection::from(aux))),
            trim: crate::ir::SampleTrim::NONE,
        };
        let added = {
            let state = self
                .tracks
                .get_mut(index)
                .ok_or(Error::InvalidInput("unknown track"))?;
            let Some(mapped) = state.mapped.as_mut() else {
                return Ok(());
            };
            if let Some(length_size) = mapped.length_size {
                normalise_nal_lengths(length_size, &mut sample)?;
            }
            if mapped.live {
                None
            } else {
                let fragmented = self.reader.is_fragmented() == Some(true);
                refine_legacy_config(&mut mapped.spec.config, &sample);
                if mapped.spec.padding.is_none()
                    && matches!(mapped.spec.config, CodecConfig::MpegAudio { .. })
                    && let Some(padding) = crate::media::lame_padding(&sample.data)
                {
                    mapped.spec.padding = padding;
                }
                if mapped.in_band && fragmented {
                    mapped.tagger = InbandDisplayTagger::new(&mapped.spec.config, &mapped.extra);
                }
                if fragmented && matches!(mapped.spec.config, CodecConfig::Hevc { .. }) {
                    mapped.spec.hdr10plus =
                        broadcast_bitstream::Hdr10PlusMetadata::sample_carries_hdr10plus(
                            &sample.data,
                        );
                }
                mapped.live = true;
                Some(mapped.spec.clone())
            }
        };
        if let Some(spec) = added {
            self.events.push_back(DemuxEvent::TrackAdded(spec));
            self.maybe_resolve();
        }
        if let Some(state) = self.tracks.get_mut(index)
            && let Some(mapped) = state.mapped.as_mut()
            && let Some(tagger) = mapped.tagger.as_mut()
        {
            tagger.tag(&mut sample);
        }
        self.events.push_back(DemuxEvent::Sample {
            track_id: sref.track.into(),
            sample,
        });
        Ok(())
    }

    fn track_position(&self, id: isobmff::TrackId) -> Option<usize> {
        self.tracks.iter().position(|state| state.id == id)
    }
}

fn sample_end(sref: &SampleRef) -> u64 {
    sref.offset.saturating_add(u64::from(sref.size))
}

impl Mp4Demux {
    fn buf_end(&self) -> u64 {
        self.buf_start
            .saturating_add(u64::try_from(self.buf.len()).unwrap_or(u64::MAX))
    }

    fn discard_to(&mut self, absolute: u64) {
        let keep = u64::try_from(isobmff::box_types::MAX_BOX_HEADER_SIZE).unwrap_or(0);
        let target = absolute.saturating_sub(keep);
        if target <= self.buf_start {
            return;
        }
        let drop = target - self.buf_start;
        let drop = usize::try_from(drop)
            .unwrap_or(usize::MAX)
            .min(self.buf.len());
        if drop == 0 {
            return;
        }
        self.buf.advance(drop);
        self.buf_start = self
            .buf_start
            .saturating_add(u64::try_from(drop).unwrap_or(u64::MAX));
    }

    fn supply_structure_from_buffer(&mut self, range: ByteRange) -> Result<bool> {
        if range.offset < self.buf_start {
            return Err(Error::InvalidInput(
                "MP4 structure range lies behind the sequential buffer",
            ));
        }
        let rel = usize::try_from(range.offset - self.buf_start).unwrap_or(usize::MAX);
        if rel > self.buf.len() {
            if self.eof {
                self.truncate_structure(range);
                return Ok(true);
            }
            return Ok(false);
        }
        let available = self.buf.len() - rel;
        let wanted = usize::try_from(range.len).unwrap_or(usize::MAX);
        if available >= wanted {
            self.last_structure = range.offset;
            self.reader
                .supply(range.offset, &self.buf[rel..rel + wanted])?;
            self.discard_to(range.end());
            return Ok(true);
        }
        if !self.eof {
            return Ok(false);
        }
        let supplied = &self.buf[rel..];
        match self.reader.supply(range.offset, supplied) {
            Ok(()) => {
                let end = range
                    .offset
                    .saturating_add(u64::try_from(available).unwrap_or(u64::MAX));
                self.discard_to(end);
                Ok(true)
            }
            Err(err) if is_truncation_error(&err) => {
                self.truncate_structure(range);
                Ok(true)
            }
            Err(err) => Err(err.into()),
        }
    }

    fn truncate_structure(&mut self, range: ByteRange) {
        self.push_truncated(range.offset);
        self.done = true;
    }

    fn supply_structure_at_eof(&mut self, range: ByteRange) -> Result<bool> {
        match self.reader.supply(range.offset, &[]) {
            Ok(()) => Ok(true),
            Err(err) if is_truncation_error(&err) => {
                self.truncate_structure(range);
                Ok(true)
            }
            Err(err) => Err(err.into()),
        }
    }

    fn feed_at(&mut self, offset: u64, bytes: Bytes) -> Result<()> {
        let got_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let (range, is_sample) = if let Some(range) = self.structure {
            (range, false)
        } else if let Some(range) = self.batch_range {
            (range, true)
        } else {
            return Err(Error::InvalidInput(
                "Mp4Demux was fed with no outstanding byte range",
            ));
        };
        if offset != range.offset || got_len > range.len {
            return Err(Error::InputRangeMismatch {
                expected_offset: range.offset,
                expected_len: range.len,
                got_offset: offset,
                got_len,
            });
        }
        if is_sample {
            self.batch_range = None;
            self.emit_batch_bytes(range, bytes)?;
        } else {
            self.last_structure = range.offset;
            self.structure = None;
            let short = got_len < range.len;
            if short {
                self.eof = true;
            }
            match self.reader.supply(range.offset, &bytes) {
                Ok(()) => {}
                Err(err) if is_truncation_error(&err) => {
                    self.truncate_structure(range);
                }
                Err(isobmff::Error::BoxSizeUnderflow { .. })
                    if self.saw_tracks
                        && usize::try_from(range.len)
                            .is_ok_and(|len| len > isobmff::box_types::BOX_HEADER_MIN_SIZE)
                        && bytes.len()
                            >= isobmff::box_types::BOX_TYPE_SIZE + BOX_SIZE_FIELD_SIZE
                        && read_box_size(&bytes)
                            >= u32::try_from(isobmff::box_types::BOX_HEADER_MIN_SIZE)
                                .unwrap_or(u32::MAX) =>
                {
                    self.push_malformed(range.offset);
                    self.reader.end_of_unit(range.end())?;
                    self.done = false;
                }
                Err(isobmff::Error::SampleDataBudgetExceeded { .. }) => {
                    self.push_malformed(range.offset);
                    self.reader.end_of_unit(range.end())?;
                    self.done = false;
                }
                Err(err) => return Err(err.into()),
            }
        }
        let pumped = self.pump();
        if pumped.as_ref().err().is_some_and(is_truncation_error_crate) {
            self.push_truncated(range.offset);
            self.done = true;
            return Ok(());
        }
        pumped
    }

    fn emit_batch_bytes(&mut self, range: ByteRange, bytes: Bytes) -> Result<()> {
        let samples = core::mem::take(&mut self.batch);
        for sref in samples {
            let start = sref.offset.saturating_sub(range.offset);
            let end = start.saturating_add(u64::from(sref.size));
            let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
                self.push_truncated(sref.offset);
                return Ok(());
            };
            if end > bytes.len() {
                self.push_truncated(sref.offset);
                return Ok(());
            }
            let data = bytes.slice(start..end);
            self.emit_sample(sref, data)?;
        }
        Ok(())
    }

    fn feed_next(&mut self, data: &[u8]) -> Result<()> {
        if let Some(offset) = self.next_need_offset() {
            self.discard_to(offset);
        }
        let cap = usize::try_from(self.config.limits.max_need_bytes()).unwrap_or(usize::MAX);
        if self.buf.len().saturating_add(data.len()) > cap {
            return Err(Error::LimitExceeded {
                what: "mp4 sequential buffer",
                limit: u64::try_from(cap).unwrap_or(u64::MAX),
            });
        }
        self.buf.extend_from_slice(data);
        self.pump()
    }

    fn next_need_offset(&self) -> Option<u64> {
        if let Some(range) = self.structure {
            return Some(range.offset);
        }
        self.batch_range.map(|range| range.offset)
    }

    fn demand_inner(&self) -> Demand {
        if self.poisoned.is_some() {
            return Demand::saturated();
        }
        match self.source {
            SourceMode::Seekable => {
                if let Some(range) = self.structure {
                    return Demand::at(
                        range.offset,
                        usize::try_from(range.len).unwrap_or(usize::MAX),
                    );
                }
                if let Some(range) = self.batch_range {
                    return Demand::at(
                        range.offset,
                        usize::try_from(range.len).unwrap_or(usize::MAX),
                    );
                }
                Demand::saturated()
            }
            SourceMode::Sequential => {
                let cap = self.config.limits.max_need_bytes();
                let buffered = u64::try_from(self.buf.len()).unwrap_or(u64::MAX);
                if buffered >= cap {
                    return Demand::saturated();
                }
                if self.done || self.finished || self.eof {
                    return Demand::saturated();
                }
                Demand::new(usize::try_from(cap - buffered).unwrap_or(usize::MAX))
            }
        }
    }
}

fn is_truncation_error(err: &isobmff::Error) -> bool {
    matches!(
        err,
        isobmff::Error::BufferTooShort(_)
            | isobmff::Error::LargesizeBufferTooShort { .. }
            | isobmff::Error::UuidBufferTooShort { .. }
    )
}

fn read_box_size(bytes: &[u8]) -> u32 {
    let mut size = [0u8; BOX_SIZE_FIELD_SIZE];
    size.copy_from_slice(&bytes[..4]);
    u32::from_be_bytes(size)
}

fn is_truncation_error_crate(err: &Error) -> bool {
    matches!(
        err,
        Error::BufferTooShort(_)
            | Error::LargesizeBufferTooShort { .. }
            | Error::UuidBufferTooShort { .. }
    )
}

impl Stage for Mp4Demux {
    type In<'a> = Mp4Input<'a>;
    type Out = DemuxEvent;
    type Error = Error;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<()> {
        if let Some(err) = &self.poisoned {
            return Err(err.clone());
        }
        match input {
            Mp4Input::Eof => {
                if self.finished {
                    return Ok(());
                }
                self.eof = true;
                self.pump()
            }
            Mp4Input::At { offset, bytes } => {
                if self.finished {
                    return Err(Error::InvalidInput("Mp4Demux was fed after finish"));
                }
                if self.eof {
                    return Err(Error::InvalidInput("Mp4Demux was fed after Eof"));
                }
                if self.source != SourceMode::Seekable {
                    return Err(Error::InvalidInput(
                        "Mp4Input::At was fed to a sequential Mp4Demux",
                    ));
                }
                self.feed_at(offset, bytes)
            }
            Mp4Input::Next(data) => {
                if self.finished {
                    return Err(Error::InvalidInput("Mp4Demux was fed after finish"));
                }
                if self.eof {
                    return Err(Error::InvalidInput("Mp4Demux was fed after Eof"));
                }
                if self.source != SourceMode::Sequential {
                    return Err(Error::InvalidInput(
                        "Mp4Input::Next was fed to a seekable Mp4Demux",
                    ));
                }
                self.feed_next(data)
            }
            Mp4Input::SegmentEnd => {
                if self.finished {
                    return Err(Error::InvalidInput("Mp4Demux was fed after finish"));
                }
                if self.eof {
                    return Err(Error::InvalidInput("Mp4Demux was fed after Eof"));
                }
                if self.source != SourceMode::Sequential {
                    return Err(Error::InvalidInput(
                        "Mp4Input::SegmentEnd was fed to a seekable Mp4Demux",
                    ));
                }
                let result = self.segment_end();
                if let Err(err) = &result {
                    self.poisoned = Some(err.clone());
                }
                result
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
            if !is_truncation_error_crate(&err) {
                return Err(err);
            }
            let position = self
                .structure
                .or(self.batch_range)
                .map_or(self.buf_start, |range| range.offset);
            self.structure = None;
            self.batch_range = None;
            self.push_truncated(position);
            self.done = true;
        }
        self.finish_flush();
        Ok(())
    }

    fn demand(&self) -> Demand {
        self.demand_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::DemuxEvent;

    const INIT: &str = "hls/cmaf-fmp4/init.mp4";

    #[test]
    fn init_tracks_are_available_before_finish() {
        let bytes = test_fixtures::read(INIT);
        let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
        demux
            .feed(Mp4Input::Next(&bytes), Timestamp::ZERO)
            .expect("feed init");
        let before: Vec<TrackSpec> = demux.init_tracks().cloned().collect();
        assert!(!before.is_empty(), "the moov announces its tracks");

        demux.finish().expect("finish");
        let mut announced: Vec<TrackSpec> = Vec::new();
        while let Some(event) = demux.poll() {
            if let DemuxEvent::TrackAdded(spec) = event {
                announced.push(spec);
            }
        }
        assert_eq!(announced, before, "TrackAdded carries the init's own specs");
    }

    #[test]
    fn buffered_reports_len_and_capacity() {
        let bytes = test_fixtures::read(INIT);
        let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
        demux
            .feed(Mp4Input::Next(&bytes), Timestamp::ZERO)
            .expect("feed init");
        let (len, capacity) = demux.buffered();
        assert!(capacity >= len);
        demux
            .feed(Mp4Input::SegmentEnd, Timestamp::ZERO)
            .expect("segment end");
        assert_eq!(demux.buffered(), (0, 0), "the buffer is released");
    }
}
