//! Bounded-memory file playout: a TS/FLV/MP4/PS/Matroska file is fed to an
//! incremental demuxer, merged across tracks in presentation order and released
//! under a wall-clock pacing gate, so live memory is a function of
//! [`FileStreamLimits`] and never of the file's length (issue #4057).
//!
//! Sans-IO: the caller owns the file handle, feeds the bytes the session asks
//! for (a forward chunk, an absolute byte range, or end of input) and answers
//! [`FileDemand::Rewind`] by seeking back to offset 0. The session never opens
//! a file and never reads a clock beyond the wall-clock instants pacing
//! compares against.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use broadcast_common::clock33::{WRAP_33BIT, unwrap_delta};
use broadcast_common::{Demand, Stage, Timestamp};
use bytes::Bytes;
use transmux::matroska_demux::{MatroskaDemux, MatroskaDemuxConfig, MatroskaInput};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::{
    DemuxEvent, Sample, StreamingFlvDemux, StreamingPsDemux, StreamingTsDemux, TrackId, TrackSpec,
};

use crate::file_source::{
    ADJACENT_PAIR_LEN, FileReaderError, MAX_SAMPLES_PER_DRAIN, PassRange, advance_offsets,
    secs_to_duration, ticks_to_duration,
};
use crate::ingress::{ProgramId, SessionEvent};
use crate::trunk::RetentionClass;

/// One mebibyte in bytes, the unit of every byte-sized limit here.
const BYTES_PER_MIB: usize = 1 << 20;

/// Default cap on demuxed sample bytes held ahead of the pacing clock: 8 MiB.
/// Above the few hundred milliseconds of buffering any frame stays ahead by,
/// small enough that a large file's playout footprint is bounded by it.
const DEFAULT_MAX_QUEUED_BYTES: usize = 8 * BYTES_PER_MIB;

/// Default media time held ahead of the pacing clock: 2 s. Long enough to ride
/// out a slow disk read between chunks without starving the pacer, short
/// enough that a burst cannot dump seconds of media into a ring.
const DEFAULT_READ_AHEAD: Duration = Duration::from_secs(2);

/// Default how far one track may trail another before the merge stops waiting
/// for it: 1 s. A sparse track (a data or caption PID at one sample a minute)
/// must never stall the dense tracks behind a queue that stays empty.
const DEFAULT_INTERLEAVE_SKEW: Duration = Duration::from_secs(1);

/// Default media time the session waits at the start of a pass for every track
/// to appear before announcing the program: 10 s.
const DEFAULT_START_WINDOW: Duration = Duration::from_secs(10);

/// Default bytes fed before the start window closes regardless: 16 MiB.
const DEFAULT_START_WINDOW_BYTES: usize = 16 * BYTES_PER_MIB;

/// Default distinct presentation times kept to measure the frame period: 64,
/// above the deepest decoder reorder (16 frames).
const DEFAULT_PTS_WINDOW: usize = 64;

/// The container whose bytes the session demuxes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamContainer {
    /// MPEG-2 transport stream ([`StreamingTsDemux`]).
    MpegTs,
    /// Flash Video ([`StreamingFlvDemux`]).
    Flv,
    /// ISO base media file format, progressive or fragmented
    /// ([`Mp4Demux`]).
    Mp4,
    /// MPEG-1/2 Program Stream ([`StreamingPsDemux`]).
    MpegPs,
    /// Matroska / WebM ([`MatroskaDemux`]).
    Matroska,
}

impl StreamContainer {
    /// Stable label for this container.
    pub fn name(&self) -> &'static str {
        match self {
            StreamContainer::MpegTs => "mpeg-ts",
            StreamContainer::Flv => "flv",
            StreamContainer::Mp4 => "mp4",
            StreamContainer::MpegPs => "mpeg-ps",
            StreamContainer::Matroska => "matroska",
        }
    }
}

broadcast_common::impl_spec_display!(StreamContainer);

/// Whether the session reaches this container's bytes by absolute range
/// rather than as a forward chunk stream.
fn reads_by_range(container: StreamContainer) -> bool {
    matches!(container, StreamContainer::Mp4 | StreamContainer::Matroska)
}

/// The memory and merge bounds of a [`StreamingFileSession`]. Every field is a
/// knob the host sets; the defaults suit a live slate/filler loop.
#[derive(Debug, Clone)]
pub struct FileStreamLimits {
    /// Bytes of demuxed samples held ahead of the pacing clock. Default 8 MiB.
    pub max_queued_bytes: usize,
    /// Media time held ahead of the pacing clock. Default 2 s.
    pub read_ahead: Duration,
    /// How far one track may trail another in the file before the merge
    /// stops waiting for it. Default 1 s.
    pub interleave_skew: Duration,
    /// Media time the session waits at the start of a pass for every track
    /// to appear before announcing. Default 10 s.
    pub start_window: Duration,
    /// Bytes fed before the start window closes regardless. Default 16 MiB.
    pub start_window_bytes: usize,
    /// Distinct presentation times kept to measure the frame period.
    /// Default 64, above the deepest decoder reorder (16 frames).
    pub pts_window: usize,
    /// The MP4 stage's caller policy: the byte-range read window and the
    /// input's total length. Defaults to one sample per request and no known
    /// total length; a file playout sets the length so the stage can reach
    /// end of input.
    pub mp4: Mp4DemuxConfig,
    /// The Matroska stage's caller policy: the hostile-input caps and the
    /// input's total length, set by a file playout for the same reason.
    pub matroska: MatroskaDemuxConfig,
}

impl Default for FileStreamLimits {
    fn default() -> Self {
        Self {
            max_queued_bytes: DEFAULT_MAX_QUEUED_BYTES,
            read_ahead: DEFAULT_READ_AHEAD,
            interleave_skew: DEFAULT_INTERLEAVE_SKEW,
            start_window: DEFAULT_START_WINDOW,
            start_window_bytes: DEFAULT_START_WINDOW_BYTES,
            pts_window: DEFAULT_PTS_WINDOW,
            mp4: Mp4DemuxConfig::default(),
            matroska: MatroskaDemuxConfig::default(),
        }
    }
}

/// One input to [`StreamingFileSession`]: the next file bytes, or the end of
/// the file.
#[non_exhaustive]
pub enum FileInput<'a> {
    /// The next chunk of the file, at the caller's current offset.
    Bytes(&'a [u8]),
    /// The bytes at `offset`, answering a [`FileDemand::BytesAt`]. `bytes`
    /// may be shorter than requested only when the file ends there.
    At {
        /// Absolute file offset of the first byte.
        offset: u64,
        /// The bytes themselves.
        bytes: Bytes,
    },
    /// The caller reached the end of the file.
    Eof,
}

impl FileInput<'_> {
    /// Stable label for this input.
    pub fn name(&self) -> &'static str {
        match self {
            FileInput::Bytes(_) => "bytes",
            FileInput::At { .. } => "at",
            FileInput::Eof => "eof",
        }
    }
}

broadcast_common::impl_spec_display!(FileInput<'_>);

/// What the session needs from the caller next.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDemand {
    /// Feed the next chunk of the file.
    Bytes,
    /// Feed the `len` bytes at absolute `offset`.
    BytesAt {
        /// Absolute file offset of the first byte.
        offset: u64,
        /// How many bytes are wanted.
        len: usize,
    },
    /// The pass has consumed its input: feed [`FileInput::Eof`].
    Eof,
    /// Seek the file to offset 0, then feed.
    Rewind,
    /// Nothing wanted until `next_due()`.
    Wait,
    /// The session has finished; drop the file.
    Done,
}

impl FileDemand {
    /// Stable label for this demand.
    pub fn name(&self) -> &'static str {
        match self {
            FileDemand::Bytes => "bytes",
            FileDemand::BytesAt { .. } => "bytes-at",
            FileDemand::Eof => "eof",
            FileDemand::Rewind => "rewind",
            FileDemand::Wait => "wait",
            FileDemand::Done => "done",
        }
    }
}

broadcast_common::impl_spec_display!(FileDemand);

/// The demuxer the session drives, one variant per supported container.
enum Demux {
    /// MPEG-2 TS.
    Ts(StreamingTsDemux),
    /// FLV.
    Flv(StreamingFlvDemux),
    /// ISO base media file format (seekable byte ranges).
    Mp4(Box<Mp4Demux>),
    /// MPEG-1/2 Program Stream.
    Ps(StreamingPsDemux),
    /// Matroska / WebM (seekable byte ranges).
    Matroska(Box<MatroskaDemux>),
}

impl Demux {
    fn new(container: StreamContainer, limits: &FileStreamLimits) -> Self {
        match container {
            StreamContainer::MpegTs => Demux::Ts(StreamingTsDemux::new()),
            StreamContainer::Flv => Demux::Flv(StreamingFlvDemux::new()),
            StreamContainer::Mp4 => Demux::Mp4(Box::new(Mp4Demux::seekable(limits.mp4.clone()))),
            StreamContainer::MpegPs => Demux::Ps(StreamingPsDemux::new()),
            StreamContainer::Matroska => {
                Demux::Matroska(Box::new(MatroskaDemux::seekable(limits.matroska.clone())))
            }
        }
    }

    fn feed(&mut self, input: FileInput<'_>) -> Result<(), FileReaderError> {
        match (self, input) {
            (Demux::Ts(demux), FileInput::Bytes(bytes)) => {
                demux.feed(bytes).map_err(FileReaderError::Demux)
            }
            (Demux::Flv(demux), FileInput::Bytes(bytes)) => {
                demux.feed(bytes).map_err(FileReaderError::Flv)
            }
            (Demux::Ps(demux), FileInput::Bytes(bytes)) => demux
                .feed(bytes, Timestamp::ZERO)
                .map_err(FileReaderError::Demux),
            (Demux::Mp4(demux), FileInput::At { offset, bytes }) => demux
                .feed(Mp4Input::At { offset, bytes }, Timestamp::ZERO)
                .map_err(FileReaderError::Demux),
            (Demux::Matroska(demux), FileInput::At { offset, bytes }) => demux
                .feed(MatroskaInput::At { offset, bytes }, Timestamp::ZERO)
                .map_err(FileReaderError::Demux),
            (demux, input) => Err(FileReaderError::WrongInput {
                container: demux.container_name(),
                input: input.name(),
            }),
        }
    }

    fn poll_event(&mut self) -> Option<DemuxEvent> {
        match self {
            Demux::Ts(demux) => demux.poll_event(),
            Demux::Flv(demux) => demux.poll_event(),
            Demux::Mp4(demux) => demux.poll(),
            Demux::Ps(demux) => demux.poll(),
            Demux::Matroska(demux) => demux.poll(),
        }
    }

    fn finish(&mut self) -> Result<(), FileReaderError> {
        match self {
            Demux::Ts(demux) => demux.finish().map_err(FileReaderError::Demux),
            Demux::Flv(demux) => {
                demux.finish();
                Ok(())
            }
            Demux::Mp4(demux) => demux.finish().map_err(FileReaderError::Demux),
            Demux::Ps(demux) => demux.finish().map_err(FileReaderError::Demux),
            Demux::Matroska(demux) => demux.finish().map_err(FileReaderError::Demux),
        }
    }

    /// The absolute byte range the stage wants next, when it reads by range.
    fn at_demand(&self) -> Option<(u64, usize)> {
        match self {
            Demux::Mp4(demux) => {
                let demand = demux.demand();
                demand.offset.map(|offset| (offset, demand.want_bytes))
            }
            Demux::Matroska(demux) => {
                let demand = demux.demand();
                demand.offset.map(|offset| (offset, demand.want_bytes))
            }
            _ => None,
        }
    }

    fn container_name(&self) -> &'static str {
        match self {
            Demux::Ts(_) => StreamContainer::MpegTs.name(),
            Demux::Flv(_) => StreamContainer::Flv.name(),
            Demux::Mp4(_) => StreamContainer::Mp4.name(),
            Demux::Ps(_) => StreamContainer::MpegPs.name(),
            Demux::Matroska(_) => StreamContainer::Matroska.name(),
        }
    }
}

/// The session's lifecycle within (and across) passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Accepting bytes for the current pass.
    Streaming,
    /// End of input seen; the pass's queues are draining before the loop
    /// decision takes effect.
    Flushing,
    /// A new pass is wanted: the caller seeks to offset 0 and feeds again.
    Rewinding,
    /// The session has finished and will emit nothing more.
    Done,
}

/// One demuxed sample waiting for its release slot.
struct QueuedSample {
    sample: Sample,
    /// Presentation time in seconds, across-track comparable.
    due: f64,
    /// Decode time in seconds, the intra-track merge tie-break.
    decode: f64,
}

/// One track's streaming state: its spec, its own output queue, and the
/// per-pass accumulators the loop offsets and frame period are derived from.
struct StreamTrack {
    spec: TrackSpec,
    timescale: f64,
    queue: VecDeque<QueuedSample>,
    queued_bytes: usize,
    /// The largest presentation time currently queued on this track.
    max_due: Option<f64>,
    /// Previous raw/widened presentation time, for the 33-bit wrap unroller.
    unroll: Option<(u64, i128)>,
    range_min: Option<i64>,
    range_max: Option<i64>,
    frame_ticks: Option<i64>,
    fallback_duration: Option<i64>,
    window: Vec<i64>,
    evicted_floor: Option<i64>,
    pts_window: usize,
}

impl StreamTrack {
    fn new(spec: TrackSpec, pts_window: usize) -> Self {
        Self {
            timescale: f64::from(spec.timescale.get()),
            spec,
            queue: VecDeque::new(),
            queued_bytes: 0,
            max_due: None,
            unroll: None,
            range_min: None,
            range_max: None,
            frame_ticks: None,
            fallback_duration: None,
            window: Vec::new(),
            evicted_floor: None,
            pts_window,
        }
    }

    fn observe(&mut self, pts: i64, duration: Option<i64>) {
        self.range_min = Some(self.range_min.map_or(pts, |min| min.min(pts)));
        self.range_max = Some(self.range_max.map_or(pts, |max| max.max(pts)));
        if self.fallback_duration.is_none()
            && let Some(duration) = duration.filter(|value| *value > 0)
        {
            self.fallback_duration = Some(duration);
        }
        self.observe_gap(pts);
    }

    /// Insert `pts` into the sorted window and, when the window overflows,
    /// compare the departing smallest value with its successor. All adjacent
    /// gaps of the distinct presentation times are measured exactly once.
    fn observe_gap(&mut self, pts: i64) {
        if let Some(floor) = self.evicted_floor
            && pts <= floor
        {
            return;
        }
        match self.window.binary_search(&pts) {
            Ok(_) => return,
            Err(position) => self.window.insert(position, pts),
        }
        while self.window.len() > self.pts_window.max(1) {
            let removed = self.window.remove(0);
            self.evicted_floor = Some(removed);
            if let Some(&next) = self.window.first() {
                self.record_gap(next.saturating_sub(removed));
            }
        }
    }

    fn record_gap(&mut self, gap: i64) {
        if gap > 0 {
            self.frame_ticks = Some(self.frame_ticks.map_or(gap, |frame| frame.min(gap)));
        }
    }

    fn drain_window(&mut self) {
        let mut frame = self.frame_ticks;
        for pair in self.window.windows(ADJACENT_PAIR_LEN) {
            let gap = pair[1].saturating_sub(pair[0]);
            if gap > 0 {
                frame = Some(frame.map_or(gap, |current| current.min(gap)));
            }
        }
        self.frame_ticks = frame;
    }

    /// This pass's span/frame as the loop arithmetic consumes it.
    fn pass_range(&mut self) -> PassRange {
        self.drain_window();
        let Some(min) = self.range_min else {
            return PassRange {
                span_ticks: 0,
                frame_ticks: 1,
                span: Duration::ZERO,
                frame: Duration::ZERO,
            };
        };
        let max = self.range_max.unwrap_or(min);
        let span_ticks = max.checked_sub(min).unwrap_or(0);
        let frame_ticks = self
            .frame_ticks
            .or(self.fallback_duration)
            .filter(|frame| *frame > 0)
            .unwrap_or(1);
        PassRange {
            span_ticks,
            frame_ticks,
            span: ticks_to_duration(span_ticks, self.spec.timescale),
            frame: ticks_to_duration(frame_ticks, self.spec.timescale),
        }
    }
}

/// A bounded-memory streaming file session: one container file played into
/// [`SessionEvent`]s, never held whole.
pub struct StreamingFileSession {
    container: StreamContainer,
    program: ProgramId,
    loop_file: bool,
    max_loops: Option<u32>,
    pace: bool,
    limits: FileStreamLimits,
    demux: Option<Demux>,
    phase: Phase,
    established: bool,
    /// The spec set last announced, so a late track or a changed spec on a
    /// later pass produces a fresh `NewProgram`.
    announced_specs: Option<Vec<TrackSpec>>,
    tracks: Vec<StreamTrack>,
    /// Per-track loop offset applied at release, keyed by track id.
    pass_offsets: HashMap<TrackId, i64>,
    /// The offsets the next pass starts with, computed at end of input.
    next_offsets: HashMap<TrackId, i64>,
    latest_seen: Option<f64>,
    /// Contiguous media time observed since the pass began: a jump larger
    /// than the start window is a discontinuity and does not advance it.
    start_progress: f64,
    bytes_fed: usize,
    tracks_resolved: bool,
    start_closed: bool,
    pass_wall_start: Option<Instant>,
    next_pass_wall_start: Option<Instant>,
    pass_first_due: Option<f64>,
    drained_this_feed: usize,
    loops_done: u32,
    loop_next: bool,
    done: bool,
}

impl StreamingFileSession {
    /// Build a session for `container`, replaying `loop_file` (bounded by
    /// `max_loops`) and pacing against the wall clock when `pace` is set.
    pub fn new(
        container: StreamContainer,
        program: ProgramId,
        loop_file: bool,
        max_loops: Option<u32>,
        pace: bool,
        limits: FileStreamLimits,
    ) -> Self {
        let demux = Demux::new(container, &limits);
        Self {
            container,
            program,
            loop_file,
            max_loops,
            pace,
            limits,
            demux: Some(demux),
            phase: Phase::Streaming,
            established: false,
            announced_specs: None,
            tracks: Vec::new(),
            pass_offsets: HashMap::new(),
            next_offsets: HashMap::new(),
            latest_seen: None,
            start_progress: 0.0,
            bytes_fed: 0,
            tracks_resolved: false,
            start_closed: false,
            pass_wall_start: None,
            next_pass_wall_start: None,
            pass_first_due: None,
            drained_this_feed: 0,
            loops_done: 0,
            loop_next: false,
            done: false,
        }
    }

    /// What the caller should do next.
    pub fn file_demand(&self) -> FileDemand {
        match self.phase {
            Phase::Done => FileDemand::Done,
            Phase::Rewinding => FileDemand::Rewind,
            Phase::Flushing => FileDemand::Wait,
            Phase::Streaming => {
                if reads_by_range(self.container) {
                    let Some(demux) = self.demux.as_ref() else {
                        return FileDemand::Eof;
                    };
                    let Some((offset, len)) = demux.at_demand() else {
                        return FileDemand::Eof;
                    };
                    if self.start_window_closed()
                        && (self.queued_bytes() >= self.limits.max_queued_bytes
                            || self.queued_media_time() >= self.limits.read_ahead.as_secs_f64())
                    {
                        return FileDemand::Wait;
                    }
                    return FileDemand::BytesAt { offset, len };
                }
                if !self.start_window_closed() {
                    // Before the program is announced the hold cap must not
                    // stop the feed short of the start window's own byte cap,
                    // or a container whose tracks all appear at the start
                    // (FLV never reports `TracksResolved`) would stall with a
                    // full hold and an open window.
                    if self.bytes_fed < self.limits.start_window_bytes {
                        return FileDemand::Bytes;
                    }
                    return FileDemand::Wait;
                }
                if self.queued_bytes() < self.limits.max_queued_bytes
                    && self.queued_media_time() < self.limits.read_ahead.as_secs_f64()
                {
                    FileDemand::Bytes
                } else {
                    FileDemand::Wait
                }
            }
        }
    }

    /// The wall-clock instant the next merge front is due, for the caller to
    /// sleep until. `None` when nothing is queued or pacing is off.
    pub fn next_due(&self) -> Option<Instant> {
        if !self.pace {
            return None;
        }
        let (_, front) = self.min_front()?;
        let base = self.pass_wall_start?;
        let first = self.pass_first_due?;
        let relative = (front.due - first).max(0.0);
        let wait = secs_to_duration(relative).ok()?;
        base.checked_add(wait)
            .or_else(|| Instant::now().checked_sub(Duration::ZERO))
    }

    /// `true` once the session has finished and nothing further will be
    /// emitted.
    pub fn drained(&self) -> bool {
        self.done
    }

    /// Acknowledge the caller's rewind: the next demand describes the new
    /// pass. A container that reads by absolute byte range needs this after it
    /// has repositioned the file and before it reads again; a forward-only
    /// container moves on by feeding its first chunk.
    pub fn rewound(&mut self) {
        if self.phase == Phase::Rewinding {
            self.phase = Phase::Streaming;
        }
    }

    fn queued_bytes(&self) -> usize {
        self.tracks.iter().map(|track| track.queued_bytes).sum()
    }

    /// The media time from the earliest queued presentation time to the
    /// latest one seen on any track.
    fn queued_media_time(&self) -> f64 {
        let Some((_, front)) = self.min_front() else {
            return 0.0;
        };
        let latest = self
            .tracks
            .iter()
            .filter_map(|track| track.max_due)
            .reduce(f64::max)
            .unwrap_or(front.due);
        (latest - front.due).max(0.0)
    }

    /// The merge front: the queued sample with the smallest presentation
    /// time, ties broken by track index then decode time — the same key
    /// `ParsedFile::build_playout` uses.
    fn min_front(&self) -> Option<(usize, &QueuedSample)> {
        let mut best: Option<(usize, &QueuedSample)> = None;
        for (index, track) in self.tracks.iter().enumerate() {
            let Some(front) = track.queue.front() else {
                continue;
            };
            let better = match best {
                None => true,
                Some((best_index, current)) => front
                    .due
                    .total_cmp(&current.due)
                    .then(index.cmp(&best_index))
                    .then(front.decode.total_cmp(&current.decode))
                    .is_lt(),
            };
            if better {
                best = Some((index, front));
            }
        }
        best
    }
}

impl StreamingFileSession {
    fn drain_events(&mut self) -> Result<(), FileReaderError> {
        loop {
            let Some(event) = self.demux.as_mut().and_then(Demux::poll_event) else {
                return Ok(());
            };
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    self.upsert_track(spec);
                }
                DemuxEvent::TrackRemoved { track_id, .. } => {
                    self.tracks.retain(|track| track.spec.track_id != track_id);
                }
                DemuxEvent::TracksResolved { .. } => self.tracks_resolved = true,
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => self.ingest_sample(track_id, sample)?,
                _ => {}
            }
        }
    }

    fn upsert_track(&mut self, spec: TrackSpec) {
        if let Some(track) = self
            .tracks
            .iter_mut()
            .find(|track| track.spec.track_id == spec.track_id)
        {
            track.spec = spec;
        } else {
            self.tracks
                .push(StreamTrack::new(spec, self.limits.pts_window));
        }
    }

    fn ingest_sample(
        &mut self,
        track_id: TrackId,
        mut sample: Sample,
    ) -> Result<(), FileReaderError> {
        let Some(index) = self
            .tracks
            .iter()
            .position(|track| track.spec.track_id == track_id)
        else {
            return Ok(());
        };
        let Some(timing) = sample.timing else {
            return Ok(());
        };
        let Some(pts) = timing.pts() else {
            return Ok(());
        };
        let widened = self.unroll_pts(index, pts)?;
        let dts = timing.dts().unwrap_or(widened);
        let offset = self.pass_offsets.get(&track_id).copied().unwrap_or(0);
        let shifted_dts = dts
            .checked_add(offset)
            .ok_or(FileReaderError::OffsetOverflow)?;
        let shifted_pts = widened
            .checked_add(offset)
            .ok_or(FileReaderError::OffsetOverflow)?;
        sample.timing = Some(timing.with_composition(shifted_dts, shifted_pts));
        let duration = timing.duration();
        let track = &mut self.tracks[index];
        track.observe(widened, duration.map(i64::from));
        let due = widened as f64 / track.timescale;
        let decode = dts as f64 / track.timescale;
        let bytes = sample.data.len();
        track.queued_bytes = track.queued_bytes.saturating_add(bytes);
        let mut position = track.queue.len();
        while position > 0 {
            let Some(previous) = track.queue.get(position - 1) else {
                break;
            };
            if previous.decode <= decode {
                break;
            }
            position -= 1;
        }
        track.queue.insert(
            position,
            QueuedSample {
                sample,
                due,
                decode,
            },
        );
        track.max_due = Some(track.max_due.map_or(due, |max| max.max(due)));
        let previous = self.latest_seen;
        self.latest_seen = Some(previous.map_or(due, |latest| latest.max(due)));
        if let Some(previous) = previous {
            let step = due - previous;
            if step > 0.0 && step <= self.limits.start_window.as_secs_f64() {
                self.start_progress += step;
            }
        }
        Ok(())
    }

    /// Widen a raw 33-bit presentation time across a wrap, when the demuxer
    /// has not already done so. Values at or above the modulus (or negative)
    /// are passed through unchanged: the demuxer's own unrolled timeline.
    fn unroll_pts(&mut self, index: usize, raw: i64) -> Result<i64, FileReaderError> {
        let Ok(raw_u) = u64::try_from(raw) else {
            self.tracks[index].unroll = None;
            return Ok(raw);
        };
        if raw_u >= WRAP_33BIT {
            self.tracks[index].unroll = None;
            return Ok(raw);
        }
        let track = &mut self.tracks[index];
        let widened = match track.unroll {
            Some((previous_raw, previous_widened)) => {
                unwrap_delta(previous_widened, previous_raw, raw_u)
            }
            None => i128::from(raw_u),
        };
        track.unroll = Some((raw_u, widened));
        i64::try_from(widened).map_err(|_| FileReaderError::OffsetOverflow)
    }

    fn start_window_closed(&self) -> bool {
        if self.tracks_resolved || self.start_closed {
            return true;
        }
        if self.bytes_fed >= self.limits.start_window_bytes {
            return true;
        }
        self.start_progress >= self.limits.start_window.as_secs_f64()
    }
}

impl StreamingFileSession {
    fn feed_bytes(&mut self, bytes: &[u8]) -> Result<(), FileReaderError> {
        let Some(demux) = self.demux.as_mut() else {
            return Ok(());
        };
        demux.feed(FileInput::Bytes(bytes))?;
        self.bytes_fed = self.bytes_fed.saturating_add(bytes.len());
        self.drain_events()
    }

    fn feed_at(&mut self, offset: u64, bytes: Bytes) -> Result<(), FileReaderError> {
        let fed = bytes.len();
        let Some(demux) = self.demux.as_mut() else {
            return Ok(());
        };
        demux.feed(FileInput::At { offset, bytes })?;
        self.bytes_fed = self.bytes_fed.saturating_add(fed);
        self.drain_events()
    }

    fn feed_eof(&mut self) -> Result<(), FileReaderError> {
        let Some(mut demux) = self.demux.take() else {
            return Ok(());
        };
        demux.finish()?;
        self.demux = Some(demux);
        self.drain_events()?;
        self.demux = None;
        self.finish_pass()
    }

    /// End of the pass: decide whether the file loops, and derive the next
    /// pass's per-track offsets from this pass's measured span/frame.
    fn finish_pass(&mut self) -> Result<(), FileReaderError> {
        self.start_closed = true;
        if !self.tracks.iter().any(|track| track.range_min.is_some()) {
            return Err(FileReaderError::NoPlayableSamples);
        }
        self.loops_done = self.loops_done.saturating_add(1);
        self.loop_next = self.loop_file && self.max_loops.is_none_or(|max| self.loops_done < max);
        self.phase = Phase::Flushing;
        if !self.loop_next {
            return Ok(());
        }
        let mut ranges = Vec::with_capacity(self.tracks.len());
        for track in &mut self.tracks {
            ranges.push(track.pass_range());
        }
        let mut offsets: Vec<i64> = self
            .tracks
            .iter()
            .map(|track| {
                self.pass_offsets
                    .get(&track.spec.track_id)
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        advance_offsets(&mut offsets, &ranges)?;
        let mut next = self.pass_offsets.clone();
        for (track, offset) in self.tracks.iter().zip(offsets) {
            next.insert(track.spec.track_id, offset);
        }
        self.next_offsets = next;
        let duration = ranges
            .iter()
            .map(|range| range.span.saturating_add(range.frame))
            .fold(Duration::ZERO, Duration::max);
        if self.pace
            && let Some(base) = self.pass_wall_start
        {
            self.next_pass_wall_start = base.checked_add(duration);
        }
        Ok(())
    }

    /// Start the next pass once the current one's queues are empty.
    fn advance_pass(&mut self) {
        if self.phase != Phase::Flushing || self.tracks.iter().any(|track| !track.queue.is_empty())
        {
            return;
        }
        if !self.loop_next {
            self.phase = Phase::Done;
            self.done = true;
            return;
        }
        self.tracks.clear();
        self.pass_offsets = self.next_offsets.clone();
        self.demux = Some(Demux::new(self.container, &self.limits));
        self.phase = Phase::Rewinding;
        self.bytes_fed = 0;
        self.tracks_resolved = false;
        self.start_closed = false;
        self.latest_seen = None;
        self.start_progress = 0.0;
        self.pass_first_due = None;
        self.drained_this_feed = 0;
        self.pass_wall_start = self.next_pass_wall_start.take();
    }

    fn next_release(&mut self) -> Option<SessionEvent> {
        if self.pace && self.drained_this_feed >= MAX_SAMPLES_PER_DRAIN {
            return None;
        }
        if !self.start_window_closed() {
            return None;
        }
        let (index, due) = {
            let (index, front) = self.min_front()?;
            (index, front.due)
        };
        let latest = self.latest_seen?;
        let skew = self.limits.interleave_skew.as_secs_f64();
        let every_track_ready = self.tracks.iter().all(|track| !track.queue.is_empty());
        if !every_track_ready && !self.start_closed && due > latest - skew {
            return None;
        }
        if self.pace {
            if self.pass_first_due.is_none() {
                self.pass_first_due = Some(due);
                if self.pass_wall_start.is_none() {
                    self.pass_wall_start = Some(Instant::now());
                }
            }
            let base = self.pass_wall_start?;
            let first = self.pass_first_due?;
            let wait = secs_to_duration((due - first).max(0.0)).ok()?;
            if base.elapsed() < wait {
                return None;
            }
        }
        let track = self.tracks.get_mut(index)?;
        let item = track.queue.pop_front()?;
        track.queued_bytes = track.queued_bytes.saturating_sub(item.sample.data.len());
        if track.queue.is_empty() {
            track.max_due = None;
        } else if track.max_due.is_some_and(|max| item.due >= max) {
            track.max_due = track.queue.iter().map(|queued| queued.due).reduce(f64::max);
        }
        self.drained_this_feed = self.drained_this_feed.saturating_add(1);
        Some(SessionEvent::Sample {
            program: self.program,
            track_id: track.spec.track_id,
            retention: RetentionClass::Timed,
            sample: item.sample,
        })
    }
}

impl StreamingFileSession {
    /// Sample bytes currently held in the per-track queues, the quantity
    /// [`FileStreamLimits::max_queued_bytes`] bounds.
    pub fn queued_sample_bytes(&self) -> usize {
        self.queued_bytes()
    }
}

impl Stage for StreamingFileSession {
    type In<'a> = FileInput<'a>;
    type Out = SessionEvent;
    type Error = FileReaderError;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<(), Self::Error> {
        self.drained_this_feed = 0;
        if self.phase == Phase::Rewinding {
            self.phase = Phase::Streaming;
        }
        if self.phase != Phase::Streaming {
            return Ok(());
        }
        match input {
            FileInput::Bytes(bytes) => self.feed_bytes(bytes),
            FileInput::At { offset, bytes } => self.feed_at(offset, bytes),
            FileInput::Eof => self.feed_eof(),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        if !self.established {
            self.established = true;
            return Some(SessionEvent::Established);
        }
        if matches!(self.phase, Phase::Rewinding | Phase::Done) {
            return None;
        }
        if self.start_window_closed() && !self.tracks.is_empty() {
            let specs: Vec<TrackSpec> =
                self.tracks.iter().map(|track| track.spec.clone()).collect();
            if self.announced_specs.as_ref() != Some(&specs) {
                self.announced_specs = Some(specs.clone());
                return Some(SessionEvent::NewProgram {
                    program: self.program,
                    tracks: specs,
                });
            }
        }
        if let Some(event) = self.next_release() {
            return Some(event);
        }
        self.advance_pass();
        None
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        if self.phase == Phase::Streaming {
            self.feed_eof()?;
        }
        Ok(())
    }

    fn demand(&self) -> Demand {
        match self.file_demand() {
            FileDemand::Bytes | FileDemand::Rewind | FileDemand::Eof => Demand::new(0),
            FileDemand::BytesAt { offset, len } => Demand::at(offset, len),
            FileDemand::Wait | FileDemand::Done => Demand::saturated(),
        }
    }
}
