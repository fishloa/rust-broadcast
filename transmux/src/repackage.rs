//! fMP4/CMAF repackaging — a demux stage, an optional track filter and time
//! window, and a segmenter, driven in one pass
//! (ISO/IEC 14496-12:2026; CMAF ISO/IEC 23000-19).
//!
//! `transmux` is an any-to-any container hub: [`Mp4Demux`]
//! parses fragmented ISOBMFF/CMAF and emits [`DemuxEvent`]s, and
//! [`Segmenter`] muxes them back into CMAF media segments. *Repackaging*
//! composes those two ends with two optional filters in between — no new box
//! parsing, and no whole [`Media`] held anywhere:
//!
//! - **track-select** — [`Repackage::select_tracks`] keeps a chosen subset of
//!   tracks (by announcement position), preserving the source order and
//!   [`TrackSpec`]s. It works on either input.
//! - **trim** — [`Repackage::trim`] keeps only the samples whose *presentation
//!   time* (composition time = decode time + `composition_offset`) falls in the
//!   half-open window `[start, end)`, expressed in the **movie timescale**.
//!   The window's lower edge is snapped **back** to the preceding random-access
//!   point on the segmentation anchor track, so the output opens on a keyframe
//!   as CMAF requires (ISO/IEC 23000-19 §7.3.2.3: a CMAF Track's first media
//!   sample must be an IDR/SAP); every other track then starts from that same
//!   snapped instant, so audio and video stay aligned. Window times are on one
//!   decode-time origin common to every track (the earliest first-sample
//!   `dts`), and the resegmented output keeps each track's offset from that
//!   origin in its first `tfdt`. A window needs the whole input (it needs that
//!   origin), so it is honoured only by [`Repackage::run_media`]; the
//!   streaming [`Repackage::run`] refuses it with
//!   [`Error::RepackageWindowNeedsWholeInput`].
//! - **resegment** — [`Segmenter`] at a new target segment duration produces
//!   fresh CMAF init + media segments cut on the anchor track's keyframes.
//!
//! [`Repackage`] is the driver: it demuxes fMP4 bytes ([`Repackage::run`]) or
//! takes an already-demuxed [`Media`] ([`Repackage::run_media`]) and returns the
//! CMAF init segment plus the media segments. Neither path builds a [`Media`].
//!
//! `no_std` + `alloc`.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::timescale::{Rounding, Timescale, rescale_i64};
use broadcast_common::{Demand, Stage, Timestamp};

use crate::error::{Error, Result};
use crate::ir::{DemuxEvent, Media, Sample, Track, TrackId, TrackSpec};
use crate::media::{TimelineOrigin, relative_decode_times};
use crate::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use crate::mux_stage::{MuxInput, MuxOutput, drive_media};
use crate::segmenter::{Segmenter, choose_anchor};

/// A repackaging driver: demux fragmented MP4 → optional track-select → optional
/// trim → resegment at a new target duration → CMAF segments.
///
/// Compose it fluently; a time window is honoured by [`Repackage::run_media`]
/// (a whole input), never by the streaming [`Repackage::run`]:
///
/// ```no_run
/// use transmux::Repackage;
/// # fn f(fmp4: &[u8]) -> transmux::Result<()> {
/// let out = Repackage::new(core::time::Duration::from_secs(2)) // 2-second target segments
///     .select_tracks(&[0])               // keep only track 0 (video)
///     .run(fmp4)?;
/// let _init = out.init_segment;
/// let _media = out.media_segments;       // Vec<Vec<u8>>
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct Repackage {
    target_duration: core::time::Duration,
    select: Option<Vec<usize>>,
    trim: Option<(u64, u64)>,
}

impl Repackage {
    /// Create a repackager that resegments at `target_duration`, keeping all
    /// tracks and the full timeline unless [`Repackage::select_tracks`] /
    /// [`Repackage::trim`] are set.
    pub fn new(target_duration: core::time::Duration) -> Self {
        Self {
            target_duration,
            select: None,
            trim: None,
        }
    }

    /// Restrict the output to the given track indices (positions in the
    /// demuxer's announcement order), preserving their order.
    pub fn select_tracks(mut self, indices: &[usize]) -> Self {
        self.select = Some(indices.to_vec());
        self
    }

    /// Trim to the half-open presentation-time window `[start, end)` in the movie
    /// timescale (see the module docs for the snapping rule).
    ///
    /// A window needs the whole input, so it is honoured by
    /// [`Repackage::run_media`] only: [`Repackage::run`] returns
    /// [`Error::RepackageWindowNeedsWholeInput`].
    pub fn trim(mut self, start: u64, end: u64) -> Self {
        self.trim = Some((start, end));
        self
    }

    /// Demux fragmented MP4 `fmp4`, apply the configured track selection, and
    /// resegment.
    ///
    /// # Errors
    /// [`Error::RepackageWindowNeedsWholeInput`] if [`Repackage::trim`] was
    /// set — a streaming byte stream has no timeline origin to place a window
    /// on. Propagates [`Mp4Demux`] errors and
    /// [`Segmenter`] errors.
    pub fn run(&self, fmp4: &[u8]) -> Result<RepackageOutput> {
        if self.trim.is_some() {
            return Err(Error::RepackageWindowNeedsWholeInput);
        }
        let total_len = u64::try_from(fmp4.len()).unwrap_or(u64::MAX);
        let mut demux =
            Mp4Demux::seekable(Mp4DemuxConfig::default().with_total_len(Some(total_len)));
        let mut sink = RepackageSink::new(self);
        loop {
            let demand = demux.demand();
            let Some(offset) = demand.offset else {
                break;
            };
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            let end = start.saturating_add(demand.want_bytes).min(fmp4.len());
            let slice = fmp4.get(start..end).unwrap_or(&[]);
            demux.feed(
                Mp4Input::At {
                    offset,
                    bytes: slice.to_vec().into(),
                },
                Timestamp::ZERO,
            )?;
            sink.set_movie_timescale(demux.movie_timescale());
            sink.announce_init(&demux);
            sink.absorb(&mut demux)?;
        }
        demux.finish()?;
        sink.set_movie_timescale(demux.movie_timescale());
        sink.announce_init(&demux);
        sink.absorb(&mut demux)?;
        sink.finish()?;
        sink.outputs()
    }

    /// Apply the configured filters to an already-demuxed [`Media`] and
    /// resegment, returning the CMAF init + media segments.
    ///
    /// # Errors
    /// A window that selects no sample, or any [`Segmenter`] error.
    pub fn run_media(&self, media: &Media) -> Result<RepackageOutput> {
        // The caller already holds the whole `Media`, so the filters run as the
        // one shared whole-input pass over it — the window's presentation-time
        // boundary rule is subtle enough (origin, inclusive start, exclusive
        // end, the snap back to the preceding sync sample) that it lives in one
        // place rather than being re-derived by a streaming look-alike.
        let filtered;
        let input = if self.select.is_none() && self.trim.is_none() {
            media
        } else {
            let selected = match &self.select {
                Some(indices) => select_tracks(media, indices)?,
                None => media.clone(),
            };
            filtered = match self.trim {
                Some((start, end)) => trim(&selected, start, end)?,
                None => selected,
            };
            &filtered
        };
        let mut sink = RepackageSink::plain(self.target_duration);
        sink.set_movie_timescale(Some(input.movie_timescale));
        let outputs = drive_media(&mut sink, input, Timestamp::ZERO)?;
        Ok(collect(&outputs))
    }
}

/// Split a demux stage's queued events into the repackaging stage.
fn drain_demux(demux: &mut Mp4Demux, sink: &mut RepackageSink) -> Result<()> {
    while let Some(event) = demux.poll() {
        sink.event(event)?;
    }
    Ok(())
}

/// Fold a stage's outputs into the CMAF init segment and its media segments.
fn collect(outputs: &[MuxOutput]) -> RepackageOutput {
    let mut init_segment = Vec::new();
    let mut media_segments = Vec::new();
    for output in outputs {
        match output {
            MuxOutput::Init(bytes) => init_segment = bytes.to_vec(),
            MuxOutput::Segment(segment) => media_segments.push(segment.bytes.to_vec()),
            _ => {}
        }
    }
    RepackageOutput {
        init_segment,
        media_segments,
    }
}

/// The result of a [`Repackage`] run: the CMAF initialization segment and the
/// resegmented media segments, in order.
#[derive(Debug, Clone)]
pub struct RepackageOutput {
    /// The `ftyp` + fragmented-init `moov` initialization segment.
    pub init_segment: Vec<u8>,
    /// The `styp`/`moof`/`mdat` media segments, in output order.
    pub media_segments: Vec<Vec<u8>>,
}

impl RepackageOutput {
    /// Concatenate the init segment and every media segment into one contiguous
    /// fragmented-MP4 byte stream — the form the streaming MP4 stage re-parses.
    pub fn to_contiguous(&self) -> Vec<u8> {
        let total =
            self.init_segment.len() + self.media_segments.iter().map(Vec::len).sum::<usize>();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&self.init_segment);
        for seg in &self.media_segments {
            out.extend_from_slice(seg);
        }
        out
    }

    /// Number of emitted media segments.
    pub fn segment_count(&self) -> usize {
        self.media_segments.len()
    }
}

/// The segmentation anchor track within a [`Media`]: the first video track
/// (any [`CodecConfig::is_video`](crate::pipeline::CodecConfig::is_video)
/// codec — AVC, HEVC, AV1, VVC, …), else the first anchor-capable track.
///
/// Delegates to [`crate::segmenter::choose_anchor`] — the single shared
/// selection rule ([`Segmenter`] uses the same function — issue #628 fixed
/// this exact "only AVC counts as anchor" bug there) — rather than
/// maintaining a second, independently-drifting copy of the rule here. This
/// module used to check only `matches!(t.spec.config, CodecConfig::Avc {
/// .. })`, so HEVC/AV1/VVC-only media (all supported elsewhere in this
/// crate) fell through to `unwrap_or(0)`: track 0, which may be audio,
/// making segment/trim boundaries cut on audio "sync samples" instead of
/// video keyframes — on ordinary, well-formed input, no malformation
/// required (audit finding #6). An input with genuinely no anchor-capable
/// track (e.g. only section-carried data tracks) is now a real, explicit
/// error instead of a silent index-0 fallback.
fn anchor_index(media: &Media) -> Result<usize> {
    choose_anchor(media.tracks.iter().map(|t| &t.spec.config))
}

/// The composition (presentation) time of each sample of a track, in that
/// track's media timescale, on the decode-time `origin` common to every track
/// of the [`Media`]. Element `i` is `dts[i] + composition_offset[i]`, where
/// `dts[i]` is the sample's own absolute
/// `Sample::timing` (`.dts`)(crate::pipeline::Sample::dts) shifted by the origin —
/// falling back to a duration-accumulated reconstruction only for a track
/// that carries no timestamp at all.
///
/// Reading each sample's real `dts` (issue #993) rather than only ever
/// accumulating `duration` matters whenever the two can diverge: a
/// discontinuity/gap between fragments, a dropped or duplicated sample the
/// duration sum doesn't reflect, or ordinary sub-tick rounding between a
/// track's nominal per-sample duration and its measured decode-time deltas.
/// One origin for every track (issue #1021) keeps a trim window at the same
/// instant on every track: a per-track rebase would move a track that starts
/// later than the others by its start offset.
///
/// Returned as `i64` because a composition offset is signed and may push an
/// early sample's presentation time slightly negative.
fn presentation_times(track: &Track, origin: &TimelineOrigin) -> Result<Vec<i64>> {
    relative_decode_times(track, origin)?
        .into_iter()
        .zip(&track.samples)
        .map(|(dts, s)| Ok(dts + i64::from(s.composition_offset()?.unwrap_or(0))))
        .collect()
}

/// Convert a window edge given in the movie timescale to a track's media
/// timescale, rounding down (`floor`) so the half-open `[start, end)` semantics
/// are preserved without dropping a boundary sample.
fn rescale_floor(ticks: u64, from_timescale: Timescale, to_timescale: Timescale) -> i64 {
    rescale_i64(
        i64::try_from(ticks).unwrap_or(i64::MAX),
        from_timescale,
        to_timescale,
        Rounding::Floor,
    )
}

/// The index of the track whose next un-fed sample has the earliest decode
/// time, compared as exact rationals (`dts / timescale`).
///
/// The comparison cross-multiplies in `u128` (`a.timing.map(|t| t.dts) * b.timescale` against
/// `b.timing.map(|t| t.dts) * a.timescale`) rather than scaling every track onto a common LCM of
/// the timescales: the timescales come from untrusted `mdhd` boxes, and three
/// large coprime ones make that LCM exceed `u64`, so an `a / gcd * b` fold
/// panics in debug and wraps in release. A `u128` cross-multiply cannot
/// overflow for any pair of `u32` timescales. A track's timescale is a
/// [`Timescale`] (non-zero by construction), so no zero denominator ever
/// reaches this comparison. Ties go to the **lowest track index**, so the
/// order is deterministic. Returns `None` when every track's samples have been
/// consumed.
#[cfg(test)]
fn earliest_decode_time_track(media: &Media, dts: &[u128], cursors: &[usize]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (ti, track) in media.tracks.iter().enumerate() {
        if cursors[ti] >= track.samples.len() {
            continue;
        }
        let Some(current) = best else {
            best = Some(ti);
            continue;
        };
        let lhs = dts[ti] * media.tracks[current].spec.timescale.get() as u128;
        let rhs = dts[current] * track.spec.timescale.get() as u128;
        if lhs < rhs {
            best = Some(ti);
        }
    }
    best
}

/// Select a subset of tracks by their **index** (position in
/// [`Media::tracks`]), preserving the given order.
///
/// This and [`select_tracks_by`]/[`trim`] are the track-selection helpers the
/// former [`Media`] methods became: they are whole-input operations (a window
/// needs the timeline), kept for the [`Repackage`] driver and its tests, not
/// sample stages.
///
/// # Errors
/// [`Error::InvalidInput`] if `indices` is empty or names an out-of-range
/// track index.
pub fn select_tracks(media: &Media, indices: &[usize]) -> Result<Media> {
    if indices.is_empty() {
        return Err(Error::InvalidInput("select_tracks: empty track selection"));
    }
    let mut tracks = Vec::with_capacity(indices.len());
    for &i in indices {
        let t = media.tracks.get(i).ok_or(Error::InvalidInput(
            "select_tracks: track index out of range",
        ))?;
        tracks.push(t.clone());
    }
    Ok(Media::new(tracks, media.movie_timescale))
}

/// Select a subset of tracks by predicate over each [`Track`], preserving
/// source order.
///
/// # Errors
/// [`Error::InvalidInput`] if the predicate keeps no track.
pub fn select_tracks_by<F>(media: &Media, mut keep: F) -> Result<Media>
where
    F: FnMut(&Track) -> bool,
{
    let tracks: Vec<Track> = media.tracks.iter().filter(|t| keep(t)).cloned().collect();
    if tracks.is_empty() {
        return Err(Error::InvalidInput(
            "select_tracks_by: predicate kept no track",
        ));
    }
    Ok(Media::new(tracks, media.movie_timescale))
}

/// Trim to the half-open presentation-time window `[start, end)`, expressed in
/// the **movie timescale** ([`Media::movie_timescale`]).
///
/// For each track the window is rescaled into that track's media timescale and
/// every sample whose composition (presentation) time lies in `[start, end)`
/// is kept. The anchor track is back-trimmed to the nearest sync sample at or
/// before the first sample the raw window would select, and every other
/// track's lower edge moves back to that same instant.
///
/// # Errors
/// [`Error::InvalidInput`] if `start >= end`, the media has no tracks, the
/// window selects no sample on any track, or no track is anchor-capable.
pub fn trim(media: &Media, start: u64, end: u64) -> Result<Media> {
    if start >= end {
        return Err(Error::InvalidInput("trim: start must be < end"));
    }
    if media.tracks.is_empty() {
        return Err(Error::InvalidInput("trim: media has no tracks"));
    }
    let anchor = anchor_index(media)?;
    let origin = TimelineOrigin::of(&media.tracks);

    let anchor_track = &media.tracks[anchor];
    let anchor_ts = anchor_track.spec.timescale;
    let anchor_pts = presentation_times(anchor_track, &origin)?;
    let anchor_lo = rescale_floor(start, media.movie_timescale, anchor_ts);
    let anchor_hi = rescale_floor(end, media.movie_timescale, anchor_ts);
    let snapped = anchor_pts
        .iter()
        .position(|&p| p >= anchor_lo && p < anchor_hi)
        .map(|mut idx| {
            while idx > 0 && !anchor_track.samples[idx].flags.is_sync {
                idx -= 1;
            }
            idx
        });

    let mut out_tracks = Vec::with_capacity(media.tracks.len());
    let mut kept_any = false;
    for (ti, track) in media.tracks.iter().enumerate() {
        let ts = track.spec.timescale;
        let pts = presentation_times(track, &origin)?;
        let hi = rescale_floor(end, media.movie_timescale, ts);
        let start_idx = if ti == anchor {
            snapped
        } else {
            let lo = match snapped {
                Some(idx) => rescale_i64(anchor_pts[idx], anchor_ts, ts, Rounding::Floor),
                None => rescale_floor(start, media.movie_timescale, ts),
            };
            pts.iter().position(|&p| p >= lo && p < hi)
        };
        let mut kept = Vec::new();
        if let Some(start_idx) = start_idx {
            for (s, &p) in track.samples[start_idx..].iter().zip(&pts[start_idx..]) {
                if p >= hi {
                    break;
                }
                kept.push(s.clone());
            }
        }
        if !kept.is_empty() {
            kept_any = true;
        }
        let first_dts = kept
            .iter()
            .find_map(|s| s.timing.and_then(|t| t.dts()))
            .unwrap_or(0)
            .max(0) as u64;
        out_tracks.push(Track::new_at(track.spec.clone(), kept, first_dts));
    }
    if !kept_any {
        return Err(Error::InvalidInput(
            "trim: window selected no samples on any track",
        ));
    }
    Ok(Media::new(out_tracks, media.movie_timescale))
}

impl Media {
    /// Total duration of the segmentation anchor track (`anchor_index`: first
    /// video track, else the first anchor-capable track) in that track's media
    /// timescale — the denominator for how many segments a resegmentation at a
    /// given target will produce.
    ///
    /// Returns `(anchor_duration_ticks, anchor_timescale)`, or `None` for empty
    /// media (or, only on an already pathological input, no anchor-capable
    /// track).
    pub fn anchor_duration(&self) -> Option<(u64, u32)> {
        let anchor = anchor_index(self).ok()?;
        let t = &self.tracks[anchor];
        let ticks: u64 = t
            .samples
            .iter()
            .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
            .sum();
        Some((ticks, t.spec.timescale.get()))
    }
}

// ---------------------------------------------------------------------------
// The repackaging stage
// ---------------------------------------------------------------------------

/// The one stage between the demuxer and the segmenter: it resolves the kept
/// track set and feeds each kept sample on to the segmenter.
struct RepackageSink {
    target_duration: core::time::Duration,
    select: Option<Vec<usize>>,
    movie_timescale: Option<Timescale>,
    /// Kept announcements, in announcement order.
    announced: Vec<TrackSpec>,
    /// Announcements seen so far (the position `select` names).
    seen: usize,
    /// `true` once the `moov`'s track list has been announced.
    init_announced: bool,
    /// The tracks the segmenter was built for.
    kept: Vec<TrackId>,
    /// The decode-time origin every track's first `tfdt` is anchored on: the
    /// first sample fed, which a decode-time ordered feed guarantees is the
    /// earliest first sample there is.
    origin: Option<(i64, Timescale)>,
    /// Tracks whose first `tfdt` has been anchored on the shared origin.
    started: Vec<TrackId>,
    segmenter: Option<Segmenter>,
    pending: VecDeque<MuxOutput>,
}

impl RepackageSink {
    fn new(repackage: &Repackage) -> Self {
        Self {
            target_duration: repackage.target_duration,
            select: repackage.select.clone(),
            movie_timescale: None,
            announced: Vec::new(),
            seen: 0,
            init_announced: false,
            kept: Vec::new(),
            origin: None,
            started: Vec::new(),
            segmenter: None,
            pending: VecDeque::new(),
        }
    }

    /// A sink with no track filter: every announced track's samples are muxed.
    fn plain(target_duration: core::time::Duration) -> Self {
        Self {
            target_duration,
            select: None,
            movie_timescale: None,
            announced: Vec::new(),
            seen: 0,
            init_announced: false,
            kept: Vec::new(),
            origin: None,
            started: Vec::new(),
            segmenter: None,
            pending: VecDeque::new(),
        }
    }

    /// The movie timescale the init segment must be built with.
    fn set_movie_timescale(&mut self, timescale: Option<Timescale>) {
        if self.movie_timescale.is_none() {
            self.movie_timescale = timescale;
        }
    }

    /// Drain the demux stage's queued events into this stage.
    fn absorb(&mut self, demux: &mut Mp4Demux) -> Result<()> {
        drain_demux(demux, self)
    }

    /// Take what [`Stage::poll`] left queued as a [`RepackageOutput`].
    fn outputs(&mut self) -> Result<RepackageOutput> {
        let outputs: Vec<MuxOutput> = self.pending.drain(..).collect();
        Ok(collect(&outputs))
    }

    /// Translate one demux event into the muxer vocabulary.
    fn event(&mut self, event: DemuxEvent) -> Result<()> {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                self.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
            }
            DemuxEvent::Sample { track_id, sample } => self.feed(
                MuxInput::Sample {
                    track: track_id,
                    sample,
                },
                Timestamp::ZERO,
            ),
            DemuxEvent::Discontinuity { .. } => {
                self.feed(MuxInput::Discontinuity { track: None }, Timestamp::ZERO)
            }
            _ => Ok(()),
        }
    }

    /// Announce every track the `moov` already names, so the segmenter is
    /// built for the whole track set even though [`Mp4Demux`] emits a track's
    /// [`TrackAdded`](DemuxEvent::TrackAdded) only alongside its first sample.
    fn announce_init(&mut self, demux: &Mp4Demux) {
        if self.init_announced || self.segmenter.is_some() || demux.movie_timescale().is_none() {
            return;
        }
        let specs: Vec<TrackSpec> = demux.init_tracks().cloned().collect();
        if specs.is_empty() {
            return;
        }
        self.init_announced = true;
        for spec in specs {
            let index = self.seen;
            self.seen += 1;
            let keep = self
                .select
                .as_ref()
                .is_none_or(|select| select.contains(&index));
            if keep {
                self.kept.push(spec.track_id);
                self.announced.push(spec);
            }
        }
    }

    /// Build the segmenter once the first kept sample proves the track set
    /// settled.
    fn start(&mut self) -> Result<()> {
        if self.announced.is_empty() {
            return Err(Error::InvalidInput(
                "repackage: no track survives the track selection",
            ));
        }
        let movie_timescale = self.movie_timescale.ok_or(Error::InvalidInput(
            "repackage: no movie timescale announced before the first sample",
        ))?;
        choose_anchor(self.announced.iter().map(|s| &s.config))?;
        let segmenter = Segmenter::new(
            self.announced.clone(),
            movie_timescale,
            self.target_duration,
        )?;
        let init = segmenter.init_segment()?;
        self.pending
            .push_back(MuxOutput::Init(bytes::Bytes::from(init)));
        self.segmenter = Some(segmenter);
        Ok(())
    }

    /// `track`'s timescale, from its announcement.
    fn timescale(&self, track: TrackId) -> Option<Timescale> {
        self.announced
            .iter()
            .find(|s| s.track_id == track)
            .map(|s| s.timescale)
    }

    /// The origin expressed in `timescale` ticks, rounded down.
    fn origin_in(&self, timescale: Timescale) -> i64 {
        match self.origin {
            Some((ticks, from)) => rescale_i64(ticks, from, timescale, Rounding::Floor),
            None => 0,
        }
    }

    /// Feed one sample into the segmenter, anchoring the track's first `tfdt`
    /// on the shared origin (issue #1021) so a track that starts later keeps
    /// that offset instead of restarting at 0.
    fn emit(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        let timescale = self
            .timescale(track)
            .ok_or(Error::InvalidInput("repackage: unknown track"))?;
        let first = sample.timing.and_then(|t| t.dts()).map_or(0, |dts| {
            dts.saturating_sub(self.origin_in(timescale)).max(0)
        }) as u64;
        let segmenter = self
            .segmenter
            .as_mut()
            .ok_or(Error::InvalidInput("repackage: sample before any track"))?;
        if !self.started.contains(&track) {
            segmenter.set_start_decode_time(track, first)?;
            self.started.push(track);
        }
        segmenter.feed(MuxInput::Sample { track, sample }, Timestamp::ZERO)
    }
}

impl Stage for RepackageSink {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => {
                // A track the `moov` already named was announced by
                // `announce_init`; its `TrackAdded` carries the first sample's
                // refinement and replaces that provisional spec.
                if let Some(existing) = self
                    .announced
                    .iter_mut()
                    .find(|announced| announced.track_id == spec.track_id)
                {
                    *existing = *spec;
                    return Ok(());
                }
                let index = self.seen;
                self.seen += 1;
                let keep = self
                    .select
                    .as_ref()
                    .is_none_or(|select| select.contains(&index));
                if keep {
                    self.kept.push(spec.track_id);
                    self.announced.push(*spec);
                }
                Ok(())
            }
            MuxInput::Sample { track, sample } => {
                if !self.kept.contains(&track) {
                    return Ok(());
                }
                if self.origin.is_none()
                    && let Some(dts) = sample.timing.and_then(|t| t.dts())
                    && let Some(timescale) = self.timescale(track)
                {
                    self.origin = Some((dts, timescale));
                }
                if self.segmenter.is_none() {
                    self.start()?;
                }
                self.emit(track, sample)
            }
            MuxInput::Discontinuity { .. } => Ok(()),
            MuxInput::Flush => self.finish(),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(segmenter) = self.segmenter.as_mut() {
            segmenter.finish()?;
            while let Some(output) = segmenter.poll() {
                self.pending.push_back(output);
            }
        }
        Ok(())
    }

    fn demand(&self) -> Demand {
        Demand::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackId;
    use crate::ir::CodecConfig;
    use crate::pipeline::Sample;
    use alloc::vec;
    use broadcast_common::Parse;

    /// A track with `count` samples one tick apart, at `timescale`.
    fn track(timescale: u32, count: usize) -> Track {
        let samples = (0..count)
            .map(|i| {
                let dts = i as i64;
                Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(1), true)
                    .expect("valid timing")
            })
            .collect();
        Track::new(
            TrackSpec::new(
                TrackId::new(1),
                Timescale::try_new(timescale).expect("timescale"),
                CodecConfig::Aac {
                    esds: isobmff::mp4esds::EsdsBox::new(isobmff::mp4esds::ESDescriptor::new(
                        1,
                        0,
                        Some(isobmff::mp4esds::DecoderConfigDescriptor::new(
                            0x40,
                            0x05,
                            false,
                            0,
                            0,
                            0,
                            Some(isobmff::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
                        )),
                        Some(isobmff::mp4esds::SLConfigDescriptor::predefined_two()),
                    )),
                    channel_count: 2,
                    sample_rate: timescale,
                    sample_size: 16,
                },
            ),
            samples,
        )
    }

    fn media(timescales: &[u32]) -> Media {
        Media::new(
            timescales.iter().map(|&t| track(t, 4)).collect(),
            Timescale::try_new(1000).expect("non-zero timescale"),
        )
    }

    /// A minimal `mdhd` **v0** box carrying `timescale` (ISO/IEC 14496-12:2026
    /// §8.4.2). The demux boundary is where a real wire zero is read.
    fn mdhd_v0(timescale: u32) -> alloc::vec::Vec<u8> {
        let mut body = alloc::vec![0u8; 32];
        body[4..8].copy_from_slice(b"mdhd");
        body[20..24].copy_from_slice(&timescale.to_be_bytes());
        body[28..30].copy_from_slice(&0x55C4u16.to_be_bytes()); // "und"
        body
    }

    /// `dts[i]` is the running decode time *per track index*; `cursors` names
    /// the next un-fed sample of each.
    fn pick(media: &Media, dts: &[u128], cursors: &[usize]) -> Option<usize> {
        earliest_decode_time_track(media, dts, cursors)
    }

    /// The comparison is an exact rational one: a track is chosen by
    /// `dts / timescale`, not by raw tick count, so 1 tick at 1000 Hz
    /// (1 ms) beats 2 ticks at 100 Hz (20 ms) even though 1 < 2 is the other
    /// way round numerically.
    #[test]
    fn earliest_is_compared_as_seconds_not_ticks() {
        let m = media(&[1000, 100]);
        // Track 0: 1 tick @1000 Hz = 1 ms. Track 1: 2 ticks @100 Hz = 20 ms.
        assert_eq!(pick(&m, &[1, 2], &[0, 0]), Some(0));
        // Reverse the roles: now track 1 is earlier.
        assert_eq!(pick(&m, &[500, 3], &[0, 0]), Some(1));
    }

    /// A tie goes to the lowest track index, whatever the timescales are — so
    /// the resegmented output is deterministic.
    #[test]
    fn a_tie_picks_the_lowest_track_index() {
        // 5 ticks @ 5 kHz == 1 tick @ 1 kHz == 1 ms.
        let m = media(&[5000, 1000]);
        assert_eq!(pick(&m, &[5, 1], &[0, 0]), Some(0));
        // Same instant expressed the other way round still picks index 0.
        let m = media(&[1000, 5000]);
        assert_eq!(pick(&m, &[1, 5], &[0, 0]), Some(0));
        // Three-way tie.
        let m = media(&[1000, 2000, 3000]);
        assert_eq!(pick(&m, &[1, 2, 3], &[0, 0, 0]), Some(0));
    }

    /// The order is stable across a whole run: advancing the cursor of the
    /// picked track moves on to the next earliest, and the sequence is exactly
    /// what the rational comparison implies.
    #[test]
    fn the_whole_interleave_order_is_stable() {
        // Track 0: 1000 Hz, so sample i is i ms. Track 1: 2000 Hz, so sample i
        // is i/2 ms. Interleaved: t1@0, t0@0 (tie -> index 0 wins: 0 ms both),
        // then t1@0.5, t0@1, t1@1, ...
        let m = media(&[1000, 2000]);
        let mut dts = vec![0u128; 2];
        let mut cursors = vec![0usize; 2];
        let mut order = Vec::new();
        while let Some(ti) = pick(&m, &dts, &cursors) {
            order.push(ti);
            dts[ti] += 1;
            cursors[ti] += 1;
        }
        assert_eq!(order, vec![0, 1, 1, 0, 1, 1, 0, 0]);
    }

    /// Exhausted tracks are skipped, and `None` comes back when there is
    /// nothing left.
    #[test]
    fn exhausted_tracks_are_skipped() {
        let m = media(&[1000, 1000]);
        assert_eq!(pick(&m, &[0, 0], &[0, 4]), Some(0), "track 1 is exhausted");
        assert_eq!(pick(&m, &[0, 0], &[4, 0]), Some(1), "track 0 is exhausted");
        assert_eq!(pick(&m, &[0, 0], &[4, 4]), None, "both exhausted");
    }

    /// The huge-coprime case: three timescales whose LCM overflows `u64` still
    /// compare correctly, and the pick is the exact rational smallest.
    #[test]
    fn huge_coprime_timescales_compare_correctly() {
        const A: u32 = 4_294_967_291;
        const B: u32 = 4_294_967_279;
        const C: u32 = 4_294_967_231;
        let m = media(&[A, B, C]);
        // All at zero: tie -> index 0.
        assert_eq!(pick(&m, &[0, 0, 0], &[0, 0, 0]), Some(0));
        // Track 0 one tick is 1/A s — the *smallest* second value, since A is
        // the largest timescale — so it wins against tracks still at zero
        // ticks only in the sense of being later; here tracks 1 and 2 are at
        // 0 s, so they are earlier and index 1 wins the tie against index 2.
        assert_eq!(pick(&m, &[1, 0, 0], &[0, 0, 0]), Some(1));
        // One tick at C (the smallest timescale) is the *largest* one-tick
        // second value, so it loses to a track at zero ticks.
        assert_eq!(pick(&m, &[0, 0, 1], &[0, 0, 0]), Some(0));
        // A track still at zero always beats one that has advanced, whatever
        // the timescales — so index 0 (untouched) wins.
        assert_eq!(pick(&m, &[0, 1, 1], &[0, 0, 0]), Some(0));
        // With track 0 exhausted, one tick at B beats one tick at C (B > C,
        // so 1/B s < 1/C s).
        assert_eq!(pick(&m, &[0, 1, 1], &[4, 0, 0]), Some(1));
        // And index 0 with one tick at A beats both (1/A s is the smallest).
        assert_eq!(pick(&m, &[1, 1, 1], &[0, 0, 0]), Some(0));
        // The LCM of A, B and C is ~7.9e28, far past u64::MAX: the old fold
        // overflowed here, this comparison must not.
        let lcm_overflows = (A as u128) * (B as u128) * (C as u128);
        assert!(lcm_overflows > u64::MAX as u128);
    }

    /// A zero timescale is an error at the boundary that reads it, never a
    /// silent 1 Hz: a wire `mdhd.timescale` of `0` makes the box fail to parse
    /// with a typed error naming the field, and a zero movie timescale
    /// (`mvhd.timescale`) is refused the same way — so the interleave
    /// comparison and `trim` can never see a zero denominator.
    #[test]
    fn a_zero_timescale_media_is_a_typed_error_not_flattened_to_one() {
        let err = isobmff::init_segment::MediaHeaderBox::parse(&mdhd_v0(0))
            .expect_err("a wire mdhd.timescale of 0 must not parse");
        assert_eq!(
            err,
            isobmff::Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "timescale",
                value: 0,
                reason: "must be non-zero",
            })
        );
        let mut mvhd = alloc::vec![0u8; 108];
        mvhd[4..8].copy_from_slice(b"mvhd");
        mvhd[20..24].copy_from_slice(&0u32.to_be_bytes()); // timescale = 0
        mvhd[104..108].copy_from_slice(&2u32.to_be_bytes()); // next_track_id
        let err = isobmff::init_segment::MovieHeaderBox::parse(&mvhd)
            .expect_err("a wire mvhd.timescale of 0 must not parse");
        assert_eq!(
            err,
            isobmff::Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "timescale",
                value: 0,
                reason: "must be non-zero",
            })
        );
    }
}
