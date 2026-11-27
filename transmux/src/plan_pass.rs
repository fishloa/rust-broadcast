//! The plan pass: record a progressive sample plan from the mux input.
//!
//! A progressive (non-fragmented) MP4 fixes every sample's size and absolute
//! offset before its first sample byte is written, because `stsz`/`stco` sit in
//! a `moov` that precedes the `mdat` (fast start) or follows it. [`PlanPass`]
//! is the first of the two passes a replayable input gets: fed the same
//! [`MuxInput`] stream the muxer will be fed, it records each sample's
//! descriptor into an [`isobmff::plan::PlanBuilder`] and yields the
//! [`OwnedPlan`] at [`Stage::finish`]. Sample bytes
//! are never held: a sample's coded payload is consumed for its length only.
//!
//! The per-track timing rules are the ones the former whole-input
//! progressive muxer applied (ISO/IEC 14496-12:2026 §8.6): `stts` carries the
//! gap to the next sample's decode time (its own duration for the last sample,
//! or where the next decode time goes backwards), `ctts` carries
//! `pts - dts`, `stss` carries the sync flags, and an `elst` (§8.6.6) delays a
//! track that starts after the earliest one by an empty edit and skips its
//! composition lead-in.
//!
//! A track's samples must be uniformly timed (every sample carries a `dts`) or
//! uniformly untimed (none does, e.g. a section-carried track): a plan pass
//! reports the mix as [`Error::UnsupportedFeature`] instead of guessing where
//! the two halves of the track meet on the shared timeline. No demuxer in this
//! workspace produces one.
//!
//! A [`MuxInput::Discontinuity`] needs no out-of-band marker in a progressive
//! file: `stts` states a decode time only as the sum of the preceding samples'
//! durations (§8.6.1.1), so a break that moves a track's time backwards is
//! expressed by rebasing its positions onto the continuing timeline
//! (`BreakRebase`) — the first post-break sample continues from the last
//! planned one's end and every later sample keeps its spacing. A forward jump
//! needs no rebase: its distance is written as the preceding sample's duration,
//! so the jump is stated as given. A backward step inside one track that no
//! signalled break named is continued the same way: `stts` has no way to state
//! it, and truncating the track to the first source is worse than continuing.
//!
//! Spec: ISO/IEC 14496-12:2026 §8.5-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::len::fit_u32;
use broadcast_core::timescale::{Rounding, Timescale, rescale, rescale_u64};

use crate::error::{Error, Result};
use crate::ir::{Sample, TrackId, TrackSpec};
use crate::media::TimelineOrigin;
use crate::mux_stage::MuxInput;
use crate::rebase::BreakRebase;
use isobmff::plan::{DEFAULT_DESCRIPTION_INDEX, OwnedPlan, PlanBuilder, PlanSample};
use isobmff::timing::EditListEntry;
use isobmff::{Limits, TrackId as IsoTrackId};

/// `elst` `media_time` of an empty edit (ISO/IEC 14496-12:2026 §8.6.6).
const ELST_EMPTY_EDIT: i64 = -1;
/// `elst` `media_rate_integer` for normal-speed playback (§8.6.6).
const ELST_NORMAL_RATE: i16 = 1;
/// Longest edit list a plan carries: an optional empty edit plus the media edit.
const EDIT_LIST_MAX_ENTRIES: usize = 2;

/// Whether a track's samples carry a decode time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    /// Every sample carries an absolute `dts`.
    Timed,
    /// No sample carries a `dts`; the track starts at the timeline origin and
    /// advances by each sample's own duration.
    Untimed,
}

/// The last sample of a track, held until the next one fixes its `stts` gap.
#[derive(Debug, Clone, Copy)]
struct PendingSample {
    position: i64,
    duration: u32,
    composition_offset: i32,
    sync: bool,
    size: u32,
}

/// One announced track: its spec and the aggregates a plan needs.
struct TrackState {
    spec: TrackSpec,
    kind: Option<SampleKind>,
    /// The first sample's absolute decode time (meaningful only for
    /// [`SampleKind::Timed`]).
    first_dts: i64,
    /// The running relative position of an untimed track's next sample.
    position: i64,
    /// The end of the track's last planned sample (its position plus its
    /// duration, in the track's media timescale) — the point a post-break
    /// sample continues from.
    last_end: Option<i64>,
    /// The offset this track's sample positions carry after a break that moved
    /// time backwards, so the `stts` timeline continues instead of stepping
    /// back.
    rebase: BreakRebase,
    /// A break named this track and its first sample after it is still to
    /// come: that sample fixes [`Self::rebase`] when the break moved time
    /// backwards (a forward jump is written as given).
    awaiting_break: bool,
    prev: Option<PendingSample>,
    /// Σ `stts` deltas, in the track's media timescale.
    media_duration: u64,
    /// Earliest sample presentation time (`dts` + `ctts`, before the timeline
    /// origin is subtracted).
    pres_start: i64,
    /// Latest sample presentation end.
    pres_end: i64,
    has_presentation: bool,
}

/// One track's finished timing: what the plan's `mdhd`/`tkhd`/`elst` carry.
struct TrackTiming {
    start_decode_time: u64,
    media_duration: u64,
    presentation_duration: u64,
    edits: Vec<EditListEntry>,
}

/// Records a progressive sample plan from the [`MuxInput`] stream.
///
/// Announce every track with [`MuxInput::Track`] before its first sample (the
/// track set is fixed at the first sample, as it is for the muxer the plan is
/// built for), feed the samples with [`MuxInput::Sample`] in decode order per
/// track, then [`Stage::finish`] and take the [`OwnedPlan`] from
/// [`Stage::poll`].
pub struct PlanPass {
    movie_timescale: Timescale,
    limits: Limits,
    tracks: Vec<TrackState>,
    builder: Option<PlanBuilder>,
    ready: Option<OwnedPlan>,
    finished: bool,
}

impl PlanPass {
    /// A plan pass whose movie (`mvhd`) timescale is `movie_timescale`, which
    /// the edit lists and the presentation durations are expressed in.
    pub fn new(movie_timescale: impl Into<Timescale>) -> Self {
        Self {
            movie_timescale: movie_timescale.into(),
            limits: Limits::default(),
            tracks: Vec::new(),
            builder: None,
            ready: None,
            finished: false,
        }
    }

    fn announce(&mut self, spec: TrackSpec) -> Result<()> {
        if self.builder.is_some() {
            return match self
                .tracks
                .iter()
                .find(|t| t.spec.track_id == spec.track_id)
            {
                Some(existing) if existing.spec.timescale == spec.timescale => Ok(()),
                _ => Err(Error::UnsupportedFeature(
                    "progressive: the track set is fixed at the first sample",
                )),
            };
        }
        match self
            .tracks
            .iter_mut()
            .find(|t| t.spec.track_id == spec.track_id)
        {
            Some(existing) => existing.spec = spec,
            None => self.tracks.push(TrackState {
                spec,
                kind: None,
                first_dts: 0,
                position: 0,
                last_end: None,
                rebase: BreakRebase::NONE,
                awaiting_break: false,
                prev: None,
                media_duration: 0,
                pres_start: 0,
                pres_end: 0,
                has_presentation: false,
            }),
        }
        Ok(())
    }

    fn ensure_builder(&mut self) -> Result<()> {
        if self.builder.is_some() || self.tracks.is_empty() {
            return Ok(());
        }
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        let setups = crate::pipeline::track_setups(&specs)?;
        self.builder = Some(PlanBuilder::new(setups, self.limits));
        Ok(())
    }

    fn push_sample(&mut self, track: TrackId, sample: &Sample) -> Result<()> {
        let index = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track)
            .ok_or(Error::InvalidInput(
                "progressive: sample for an unannounced track",
            ))?;
        self.ensure_builder()?;
        let size = fit_u32(sample.data.len(), "stsz entry_size")?;
        let composition_offset = sample.composition_offset()?.unwrap_or(0);
        let duration = sample.timing.and_then(|t| t.duration()).unwrap_or(0);
        let dts = sample.timing.and_then(|t| t.dts());
        let sync = sample.flags.is_sync;

        let track_id = IsoTrackId::new(track.value());
        if let Some(state) = self.tracks.get_mut(index)
            && state.awaiting_break
        {
            state.awaiting_break = false;
            if let (Some(last_end), Some(raw)) = (state.last_end, dts) {
                // A break that moves time backwards continues the `stts`
                // timeline; a forward jump is written as given. A stray
                // residual step is caught by the per-sample check below.
                state.rebase = BreakRebase::backwards(last_end, raw);
            }
        }
        let state = self
            .tracks
            .get_mut(index)
            .ok_or(Error::InvalidInput("progressive: plan track is missing"))?;
        let kind = match state.kind {
            None => {
                let kind = if dts.is_some() {
                    SampleKind::Timed
                } else {
                    SampleKind::Untimed
                };
                state.kind = Some(kind);
                if let Some(d) = dts {
                    state.first_dts = d;
                    state.position = d;
                }
                kind
            }
            Some(kind) => kind,
        };
        if (kind == SampleKind::Timed) != dts.is_some() {
            return Err(Error::UnsupportedFeature(
                "progressive: a track's samples must be all timed or all untimed",
            ));
        }
        // A `Timed` track's every sample carries a `dts` (checked above).
        let raw = dts.unwrap_or(0);
        let mut position = match kind {
            SampleKind::Timed => state.rebase.apply(raw),
            SampleKind::Untimed => state.position,
        };
        if kind == SampleKind::Untimed {
            state.position = position.saturating_add(i64::from(duration));
        } else if let Some(prev) = state.prev
            && position < prev.position
        {
            // `stts` cannot step back, and a live source can switch origin
            // without a signalled break reaching every track: continue this
            // track from the last sample's end rather than truncating it.
            let target = prev.position.saturating_add(i64::from(prev.duration));
            state.rebase = BreakRebase::continue_from(target, raw);
            position = target;
        }
        state.last_end = Some(position.saturating_add(i64::from(duration)));
        let presentation = position.saturating_add(i64::from(composition_offset));
        let presentation_end = presentation.saturating_add(i64::from(duration));
        if !state.has_presentation {
            state.pres_start = presentation;
            state.pres_end = presentation_end;
            state.has_presentation = true;
        } else {
            state.pres_start = state.pres_start.min(presentation);
            state.pres_end = state.pres_end.max(presentation_end);
        }
        let previous = state.prev.replace(PendingSample {
            position,
            duration,
            composition_offset,
            sync,
            size,
        });
        if let Some(prev) = previous {
            let delta = if position >= prev.position {
                u32::try_from(position - prev.position).unwrap_or(prev.duration)
            } else {
                prev.duration
            };
            state.media_duration = state.media_duration.saturating_add(u64::from(delta));
            let builder = self
                .builder
                .as_mut()
                .ok_or(Error::InvalidInput("progressive: no plan builder"))?;
            builder.push(track_id, plan_sample(delta, prev))?;
        }
        Ok(())
    }

    /// Finish every track's last sample and build the plan.
    fn finalize(&mut self) -> Result<()> {
        if self.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package media with no tracks"));
        }
        self.ensure_builder()?;
        for index in 0..self.tracks.len() {
            let track_id = IsoTrackId::new(
                self.tracks
                    .get(index)
                    .map(|t| t.spec.track_id.value())
                    .unwrap_or(0),
            );
            let last = self.tracks.get_mut(index).and_then(|t| {
                let prev = t.prev.take()?;
                t.media_duration = t.media_duration.saturating_add(u64::from(prev.duration));
                Some(prev)
            });
            if let Some(prev) = last {
                let builder = self
                    .builder
                    .as_mut()
                    .ok_or(Error::InvalidInput("progressive: no plan builder"))?;
                builder.push(track_id, plan_sample(prev.duration, prev))?;
            }
        }

        let origin = TimelineOrigin::from_first_decode_times(self.tracks.iter().map(|t| {
            let first = match t.kind {
                Some(SampleKind::Timed) => Some(t.first_dts),
                _ => None,
            };
            (first, t.spec.timescale)
        }));
        let epoch = self.movie_timescale;
        let movie_start = self
            .tracks
            .iter()
            .filter(|t| t.has_presentation)
            .map(|t| to_movie(presentation_start(t, &origin), t.spec.timescale, epoch))
            .min()
            .unwrap_or(0);
        let timings: Vec<TrackTiming> = self
            .tracks
            .iter()
            .map(|t| track_timing(t, &origin, movie_start, epoch))
            .collect::<Result<Vec<_>>>()?;

        let builder = self
            .builder
            .as_mut()
            .ok_or(Error::InvalidInput("progressive: no plan builder"))?;
        for (state, timing) in self.tracks.iter().zip(&timings) {
            let track_id = IsoTrackId::new(state.spec.track_id.value());
            builder.set_start_decode_time(track_id, timing.start_decode_time)?;
            builder.set_durations(
                track_id,
                timing.media_duration,
                timing.presentation_duration,
            )?;
            builder.set_edits(track_id, timing.edits.clone())?;
        }
        let builder = self
            .builder
            .take()
            .ok_or(Error::InvalidInput("progressive: no plan builder"))?;
        self.ready = Some(builder.finish()?);
        Ok(())
    }
}

impl Stage for PlanPass {
    type In<'a> = MuxInput;
    type Out = OwnedPlan;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => self.announce(*spec),
            MuxInput::Sample { track, sample } => self.push_sample(track, &sample),
            MuxInput::Discontinuity { track } => {
                for state in &mut self.tracks {
                    if track.is_none_or(|broken| state.spec.track_id == broken) {
                        state.awaiting_break = true;
                    }
                }
                Ok(())
            }
            MuxInput::TrackRemoved { .. } => Err(Error::UnsupportedFeature(
                "progressive: the plan names every track the file header describes",
            )),
            MuxInput::Flush => Ok(()),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.take()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.finalize()
    }
}

fn plan_sample(duration: u32, prev: PendingSample) -> PlanSample {
    PlanSample {
        duration,
        composition_offset: prev.composition_offset,
        sync: prev.sync,
        size: prev.size,
        description_index: DEFAULT_DESCRIPTION_INDEX,
    }
}

/// A track's presentation start relative to the timeline origin.
fn presentation_start(track: &TrackState, origin: &TimelineOrigin) -> i64 {
    track.pres_start.saturating_sub(track_origin(track, origin))
}

/// A track's presentation end relative to the timeline origin.
fn presentation_end(track: &TrackState, origin: &TimelineOrigin) -> i64 {
    track.pres_end.saturating_sub(track_origin(track, origin))
}

/// The timeline origin in a track's own timescale; an untimed track starts at
/// zero by definition.
fn track_origin(track: &TrackState, origin: &TimelineOrigin) -> i64 {
    match track.kind {
        Some(SampleKind::Timed) => origin.in_timescale(track.spec.timescale),
        _ => 0,
    }
}

/// `ticks` in the movie timescale, rounded down.
fn to_movie(ticks: i64, timescale: Timescale, movie_timescale: Timescale) -> i128 {
    rescale(
        i128::from(ticks),
        timescale,
        movie_timescale,
        Rounding::Floor,
    )
}

fn track_timing(
    track: &TrackState,
    origin: &TimelineOrigin,
    movie_start: i128,
    movie_timescale: Timescale,
) -> Result<TrackTiming> {
    let timescale = track.spec.timescale;
    let base = track_origin(track, origin);
    let first_dts = match track.kind {
        Some(SampleKind::Timed) => track.first_dts.saturating_sub(base),
        _ => 0,
    };
    let start_decode_time = u64::try_from(first_dts).map_err(|_| {
        Error::InvalidInput("progressive: a track starts before the timeline origin")
    })?;
    let plain_duration = rescale_u64(
        track.media_duration,
        timescale,
        movie_timescale,
        Rounding::TowardZero,
    );
    if !track.has_presentation {
        return Ok(TrackTiming {
            start_decode_time,
            media_duration: track.media_duration,
            presentation_duration: plain_duration,
            edits: Vec::new(),
        });
    }
    let pres_start = presentation_start(track, origin);
    let pres_end = presentation_end(track, origin);
    let empty = (to_movie(pres_start, timescale, movie_timescale) - movie_start).max(0) as u64;
    let media_time = (pres_start - first_dts).max(0);
    let segment_duration = (to_movie(pres_end, timescale, movie_timescale)
        - to_movie(pres_start, timescale, movie_timescale))
    .max(0) as u64;
    if empty == 0 && media_time == 0 {
        return Ok(TrackTiming {
            start_decode_time,
            media_duration: track.media_duration,
            presentation_duration: plain_duration,
            edits: Vec::new(),
        });
    }
    let mut edits = Vec::with_capacity(EDIT_LIST_MAX_ENTRIES);
    if empty > 0 {
        edits.push(EditListEntry {
            segment_duration: empty,
            media_time: ELST_EMPTY_EDIT,
            media_rate_integer: ELST_NORMAL_RATE,
            media_rate_fraction: 0,
        });
    }
    edits.push(EditListEntry {
        segment_duration,
        media_time,
        media_rate_integer: ELST_NORMAL_RATE,
        media_rate_fraction: 0,
    });
    Ok(TrackTiming {
        start_decode_time,
        media_duration: track.media_duration,
        presentation_duration: empty.saturating_add(segment_duration),
        edits,
    })
}
