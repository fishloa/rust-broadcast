//! Timeline splice / concatenation → server-side ad insertion (SSAI), as a
//! sample stage over the media IR (issue #475).
//!
//! Where [`crate::rebase`] conditions a *single* timeline (rebase-to-zero,
//! offset, gap insertion), this module *joins* timelines on one monotonic
//! decode clock:
//!
//! - [`SplicePlan::concat`] — append `b` after `a` on a shared timeline: for
//!   each matched track, `b`'s samples follow `a`'s, rebased so the reference
//!   track lands exactly on `a`'s end decode time.
//! - [`SplicePlan::insert`] — SSAI: play `base` up to a splice time, insert
//!   `ad`, then resume the remainder of `base` shifted forward by `ad`'s span.
//!
//! A plan holds no samples: the per-track cut indices and tick shifts plus the
//! [`SplicePoint`]s to signal. [`SpliceStage`] applies it per sample, so the
//! transform never holds a whole input. The former whole-input
//! `concat`/`splice_insert` functions are gone with it.
//!
//! # Timeline model (ISO/IEC 14496-12:2026 `tfdt`)
//!
//! [`Sample`] timing is *absolute* (media plane step 2c): each sample carries
//! its own optional decode time (DTS) and presentation time (PTS = DTS +
//! composition offset), in the track's media timescale. Each boundary's
//! absolute `dts` is read directly rather than reconstructed as `start +
//! Σ durations`, falling back to that reconstruction only where a sample
//! genuinely carries no timestamp (a section-carried sample). A splice cut
//! taken from the video track is an absolute decode time, and every other
//! track's cut is found by comparing that instant against each sample's own
//! absolute `dts` — never against a position measured from that track's own
//! start, which would skew the cut whenever the tracks start differently.
//!
//! # Rebasing the spliced-in content (issue #782)
//!
//! Two independently-demuxed assets have unrelated absolute timelines, so the
//! plan derives **one offset per splice from a single reference track** (the
//! video track if either side has one, else the first track that carries a
//! real timestamp) and applies it uniformly to every matched track, converted
//! into each track's own timescale. `dts` and `pts` shift by the same amount,
//! so composition offsets survive exactly, and a `None`-timed sample is left
//! untouched. The fresh absolute endpoints are rescaled per splice, so
//! sub-tick rounding never compounds across a chain.
//!
//! # Keyframe / RAP alignment
//!
//! [`SplicePlan::insert`] snaps the requested splice time to the nearest
//! preceding sync sample of the base's video track ([`snap_to_preceding_sync`],
//! keepable as a plan query). [`SpliceStage`] itself holds no more than its
//! caller-supplied cap of samples: without a plan it can hold one GOP while it
//! waits for a sync point, and reports [`Error::SpliceGopOverflow`] past the
//! cap.
//!
//! # Discontinuity signalling
//!
//! Each join is a media-timeline discontinuity (RFC 8216 §4.3.2.3). The plan
//! reports [`SplicePoint`]s (track id + sample index + presentation time of
//! each join) so a downstream packager marks exactly those segments.
//!
//! # Example
//!
//! ```rust
//! use transmux::{SpliceIndex, SplicePlan, SpliceStage};
//! use broadcast_common::timescale::Timescale;
//!
//! # fn plan(base: &SpliceIndex, ad: &SpliceIndex, scale: Timescale) -> transmux::Result<()> {
//! // Play `base` up to 90 000 presentation ticks (1 s of a 90 kHz track),
//! // insert `ad`, then resume `base` shifted by `ad`'s span.
//! let plan = SplicePlan::insert(base, ad, scale, 90_000)?;
//! let mut stage = SpliceStage::new(plan);
//! // Feed (TrackId, Sample) pairs in the plan's order, then finish and poll.
//! let _ = &mut stage;
//! # Ok(()) }
//! ```

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::num::NonZeroU32;
use core::ops::Deref;

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, rescale_i64};

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::{DemuxEvent, Sample, SampleTiming, TrackSpec};

/// A splice / concatenation join point in a resulting track.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SplicePoint {
    /// The track this join occurs on.
    pub track_id: TrackId,
    /// Index, within the resulting track's samples, of the first sample of the
    /// spliced-in contribution (the sample that opens the discontinuous
    /// region).
    pub sample_index: usize,
    /// Presentation time (PTS = DTS + composition_offset) of that sample, in
    /// the track's media timescale.
    pub presentation_time: u64,
}

/// One track's slice of a [`SplicePlan`]: how many samples pass unchanged,
/// how many are inserted, and the tick shifts for each run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrackSplice {
    /// The track this slice applies to.
    pub track_id: TrackId,
    /// Number of source samples emitted unchanged before the inserted run.
    pub split_index: usize,
    /// Number of inserted samples (the ad / appended contribution).
    pub insert_count: usize,
    /// The track identity the inserted samples are read from.
    pub insert_track: TrackId,
    /// The position of the inserted track in the second media's `tracks`.
    pub insert_index: usize,
    /// Ticks added to every inserted sample's `dts`/`pts`.
    pub insert_shift: i64,
    /// Ticks added to every source sample from `split_index + insert_count` on.
    pub resume_shift: i64,
}

/// One sample's planner-relevant timing and flags, with no payload: exactly
/// the fields [`SplicePlan::concat`] / [`SplicePlan::insert`] read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SpliceSample {
    /// Absolute decode timestamp, when the sample has one.
    pub dts: Option<i64>,
    /// Absolute presentation timestamp, when the sample has one.
    pub pts: Option<i64>,
    /// The sample's span in ticks, when known.
    pub duration: Option<NonZeroU32>,
    /// Whether this is a sync sample (random-access point / keyframe).
    pub is_sync: bool,
}

impl SpliceSample {
    /// Build a sample's planner-relevant fields from its [`SampleTiming`] and
    /// sync flag.
    #[must_use]
    pub fn new(timing: Option<SampleTiming>, is_sync: bool) -> Self {
        Self {
            dts: timing.and_then(|t| t.dts()),
            pts: timing.and_then(|t| t.pts()),
            duration: timing.and_then(|t| t.duration()).and_then(NonZeroU32::new),
            is_sync,
        }
    }

    fn duration_ticks(&self) -> u32 {
        self.duration.map_or(0, NonZeroU32::get)
    }
}

/// One track's planner-relevant identity and sample timings: a [`TrackSpec`]
/// plus, per sample, only what the splice planner reads.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SpliceTrack {
    /// Track identity + codec configuration (used to match tracks and pick the
    /// reference timeline).
    pub spec: TrackSpec,
    /// The track's samples, decode order, with no coded bytes.
    pub samples: Vec<SpliceSample>,
    /// Absolute decode time of the track's first sample, in its media
    /// timescale.
    pub start_decode_time: u64,
}

/// The per-track index [`SplicePlan::concat`] / [`SplicePlan::insert`] read:
/// timing and flags only, never coded sample payloads.
#[derive(Debug, Clone, Default)]
pub struct SpliceIndex {
    tracks: Vec<SpliceTrack>,
}

impl SpliceIndex {
    /// The indexed tracks, in source order.
    #[must_use]
    pub fn tracks(&self) -> &[SpliceTrack] {
        &self.tracks
    }
}

impl Deref for SpliceIndex {
    type Target = [SpliceTrack];

    fn deref(&self) -> &Self::Target {
        &self.tracks
    }
}

/// Builds a [`SpliceIndex`] one track, `DemuxEvent`, or sample timing at a
/// time, so a caller never holds the coded samples.
#[derive(Debug, Clone, Default)]
pub struct SpliceIndexBuilder {
    tracks: Vec<SpliceTrack>,
}

impl SpliceIndexBuilder {
    /// A builder with no tracks.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a track and return the index its samples attach to.
    pub fn add_track(&mut self, spec: TrackSpec, start_decode_time: u64) -> usize {
        self.tracks.push(SpliceTrack {
            spec,
            samples: Vec::new(),
            start_decode_time,
        });
        self.tracks.len() - 1
    }

    /// Append one sample's timing and sync flag to the track at `index`.
    pub fn push_sample(&mut self, index: usize, timing: Option<SampleTiming>, is_sync: bool) {
        if let Some(track) = self.tracks.get_mut(index) {
            track.samples.push(SpliceSample::new(timing, is_sync));
        }
    }

    /// Feed one demux event: `TrackAdded`/`TrackUpdated` maintain the track
    /// set, `Sample` appends its timing (never its payload).
    pub fn push_event(&mut self, event: &DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                if !self.tracks.iter().any(|t| t.spec.track_id == spec.track_id) {
                    self.add_track(spec.clone(), 0);
                }
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(track) = self
                    .tracks
                    .iter_mut()
                    .find(|t| t.spec.track_id == spec.track_id)
                {
                    track.spec = spec.clone();
                }
            }
            DemuxEvent::Sample { track_id, sample } => {
                if let Some(index) = self
                    .tracks
                    .iter()
                    .position(|t| t.spec.track_id == *track_id)
                {
                    if self.tracks[index].samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|t| t.dts())
                    {
                        self.tracks[index].start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    self.push_sample(index, sample.timing, sample.flags.is_sync);
                }
            }
            _ => {}
        }
    }

    /// Finish the index.
    #[must_use]
    pub fn build(self) -> SpliceIndex {
        SpliceIndex {
            tracks: self.tracks,
        }
    }
}

/// A computed splice over the sample stream, with the joins to signal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SplicePlan {
    tracks: Vec<TrackSplice>,
    movie_timescale: Timescale,
    points: Vec<SplicePoint>,
}

impl SplicePlan {
    /// The per-track slices, in the base media's track order.
    #[must_use]
    pub fn tracks(&self) -> &[TrackSplice] {
        &self.tracks
    }

    /// The slice for `track`, if the plan names it.
    #[must_use]
    pub fn plan_for(&self, track: TrackId) -> Option<&TrackSplice> {
        self.tracks.iter().find(|t| t.track_id == track)
    }

    /// The movie timescale the joined media carries.
    #[must_use]
    pub fn movie_timescale(&self) -> Timescale {
        self.movie_timescale
    }

    /// The join points to signal as `#EXT-X-DISCONTINUITY`.
    #[must_use]
    pub fn discontinuity_points(&self) -> &[SplicePoint] {
        &self.points
    }

    /// A copy of this plan carrying only the slice at `index`, so a driver can
    /// feed one track at a time even when several tracks share a `TrackId`.
    #[must_use]
    pub fn only(&self, index: usize) -> Option<Self> {
        let track = *self.tracks.get(index)?;
        Some(Self {
            tracks: alloc::vec![track],
            movie_timescale: self.movie_timescale,
            points: self.points.clone(),
        })
    }
}

/// The decode time of a track's first sample beyond its last (its **end decode
/// time**): the last sample's own absolute `dts` plus its `duration` when
/// known, else the duration-sum from its start.
fn track_end_decode_time(track: &SpliceTrack) -> u64 {
    if let Some(last) = track.samples.last()
        && let Some(dts) = last.dts
    {
        let dts = u64::try_from(dts).unwrap_or(0);
        return dts.saturating_add(u64::from(last.duration_ticks()));
    }
    let span: u64 = track
        .samples
        .iter()
        .map(|s| u64::from(s.duration_ticks()))
        .sum();
    track.start_decode_time.saturating_add(span)
}

/// Match `a.tracks[i]` to an index in `b`: by `track_id` first, else by index.
/// The mapping is injective; mismatched counts, an unavailable match, or an
/// incompatible codec kind/timescale is [`Error::InvalidInput`].
fn match_tracks(a: &[SpliceTrack], b: &[SpliceTrack]) -> Result<Vec<usize>> {
    if a.len() != b.len() {
        return Err(Error::InvalidInput(
            "splice: media have differing track counts",
        ));
    }
    let mut mapping = Vec::with_capacity(a.len());
    let mut used = alloc::vec![false; b.len()];
    for (i, at) in a.iter().enumerate() {
        let bj = b
            .iter()
            .enumerate()
            .find(|(j, bt)| !used[*j] && bt.spec.track_id == at.spec.track_id)
            .map(|(j, _)| j)
            .or_else(|| (!used[i]).then_some(i))
            .ok_or(Error::InvalidInput(
                "splice: no unclaimed matching track in b (non-injective match)",
            ))?;
        used[bj] = true;
        let bt = &b[bj];
        if core::mem::discriminant(&at.spec.config) != core::mem::discriminant(&bt.spec.config) {
            return Err(Error::InvalidInput(
                "splice: matched tracks have incompatible codecs",
            ));
        }
        if at.spec.timescale != bt.spec.timescale {
            return Err(Error::InvalidInput(
                "splice: matched tracks have incompatible timescales",
            ));
        }
        mapping.push(bj);
    }
    Ok(mapping)
}

/// The single track whose timeline anchors the splice offset applied uniformly
/// to every track (issue #782): the video track if either side has one, else
/// the first track that carries a real timestamp on either side.
fn pick_reference_track(a: &[SpliceTrack], b: &[SpliceTrack], mapping: &[usize]) -> Option<usize> {
    if let Some(i) = a.iter().position(|t| t.spec.config.is_video()) {
        return Some(i);
    }
    a.iter().enumerate().position(|(i, at)| {
        at.samples.iter().any(|s| s.dts.is_some())
            || b[mapping[i]].samples.iter().any(|s| s.dts.is_some())
    })
}

/// Rescale absolute ticks from one track's media timescale into another's,
/// truncating toward zero.
fn rescale_ticks(ticks: i64, from_timescale: Timescale, to_timescale: Timescale) -> i64 {
    rescale_i64(ticks, from_timescale, to_timescale, Rounding::TowardZero)
}

/// Snap a requested decode time to the nearest **preceding** sync sample of a
/// track, returning `(snapped_decode_time, sample_index)`.
///
/// `at_ticks` is an absolute decode time in `track`'s media timescale. A
/// request before the first sample snaps to index 0; a track with no samples
/// has nothing to snap to (`None`).
#[must_use]
pub fn snap_to_preceding_sync(track: &SpliceTrack, at_ticks: u64) -> Option<(u64, usize)> {
    if track.samples.is_empty() {
        return None;
    }
    let mut fallback_dts = track.start_decode_time;
    let mut best: (u64, usize) = (track.start_decode_time, 0);
    for (i, s) in track.samples.iter().enumerate() {
        let dts = s
            .dts
            .map(|d| u64::try_from(d).unwrap_or(0))
            .unwrap_or(fallback_dts);
        if dts > at_ticks {
            break;
        }
        if s.is_sync {
            best = (dts, i);
        }
        fallback_dts = dts.saturating_add(u64::from(s.duration_ticks()));
    }
    Some(best)
}

/// The decode time of `track`'s natural continuation at `split_index`, in its
/// own media timescale: the boundary sample's absolute `dts` when it has one,
/// the track's end decode time for a past-the-end split, else the duration
/// sum from the track start.
fn boundary_decode_time(track: &SpliceTrack, split_index: usize) -> u64 {
    match track.samples.get(split_index).and_then(|s| s.dts) {
        Some(dts) => u64::try_from(dts).unwrap_or(0),
        None if split_index >= track.samples.len() => track_end_decode_time(track),
        None => {
            track.start_decode_time
                + track.samples[..split_index]
                    .iter()
                    .map(|s| u64::from(s.duration_ticks()))
                    .sum::<u64>()
        }
    }
}

/// First sample index of `track` whose **absolute** decode time is at or beyond
/// `cut_ticks`; clamps to the sample count for a past-the-end cut. Compared in
/// `i128` so a negative (rebased) absolute time is simply "before the cut".
fn sample_index_at_absolute_dts(track: &SpliceTrack, cut_ticks: u64) -> usize {
    let mut acc = track.start_decode_time;
    let cut = i128::from(cut_ticks);
    for (i, s) in track.samples.iter().enumerate() {
        let absolute = match s.dts {
            Some(dts) => i128::from(dts),
            None => i128::from(acc),
        };
        if absolute >= cut {
            return i;
        }
        acc = acc.saturating_add(u64::from(s.duration_ticks()));
    }
    track.samples.len()
}

impl SplicePlan {
    /// Append `b` after `a` on a shared, contiguous, monotonic decode timeline.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the track sets are incompatible, or if a
    /// matched `b` track's first sample is not a sync sample.
    pub fn concat(a: &SpliceIndex, b: &SpliceIndex, movie_timescale: Timescale) -> Result<Self> {
        let mapping = match_tracks(a, b)?;
        for &bj in &mapping {
            if let Some(first) = b[bj].samples.first()
                && !first.is_sync
            {
                return Err(Error::InvalidInput(
                    "concat: appended track does not begin on a sync sample",
                ));
            }
        }
        let ref_shift = pick_reference_track(a, b, &mapping).map(|ri| {
            let ref_timescale = a[ri].spec.timescale;
            let join_dts_ref = track_end_decode_time(&a[ri]) as i64;
            let bt = &b[mapping[ri]];
            let incoming_ref_start = bt
                .samples
                .first()
                .and_then(|s| s.dts)
                .unwrap_or(bt.start_decode_time as i64);
            (ref_timescale, join_dts_ref, incoming_ref_start)
        });

        let mut tracks = Vec::with_capacity(a.len());
        let mut points = Vec::new();
        for (i, at) in a.iter().enumerate() {
            let bt = &b[mapping[i]];
            let insert_shift = match ref_shift {
                Some((ref_ts, join_dts_ref, incoming_ref_start)) => {
                    let track_ts = at.spec.timescale;
                    rescale_ticks(join_dts_ref, ref_ts, track_ts)
                        - rescale_ticks(incoming_ref_start, ref_ts, track_ts)
                }
                None => 0,
            };
            let split_index = at.samples.len();
            let insert_count = bt.samples.len();
            if insert_count > 0 {
                let presentation_time = match bt.samples.first().and_then(|s| s.pts) {
                    Some(p) => u64::try_from(p.saturating_add(insert_shift)).unwrap_or(0),
                    None => track_end_decode_time(at),
                };
                points.push(SplicePoint {
                    track_id: at.spec.track_id,
                    sample_index: split_index,
                    presentation_time,
                });
            }
            tracks.push(TrackSplice {
                track_id: at.spec.track_id,
                split_index,
                insert_count,
                insert_track: bt.spec.track_id,
                insert_index: mapping[i],
                insert_shift,
                resume_shift: 0,
            });
        }
        Ok(Self {
            tracks,
            movie_timescale,
            points,
        })
    }

    /// Splice `ad` into `base` at `at_ticks` (server-side ad insertion).
    ///
    /// `at_ticks` is an absolute decode time in the base video track's media
    /// timescale, snapped to the nearest preceding sync sample.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the track sets are incompatible, if `base`
    /// has no video track to align on, or if a matched `ad` track's first
    /// sample is not a sync sample.
    pub fn insert(
        base: &SpliceIndex,
        ad: &SpliceIndex,
        movie_timescale: Timescale,
        at_ticks: u64,
    ) -> Result<Self> {
        let mapping = match_tracks(base, ad)?;
        for &aj in &mapping {
            if let Some(first) = ad[aj].samples.first()
                && !first.is_sync
            {
                return Err(Error::InvalidInput(
                    "splice_insert: ad track does not begin on a sync sample",
                ));
            }
        }
        let video_idx =
            base.iter()
                .position(|t| t.spec.config.is_video())
                .ok_or(Error::InvalidInput(
                    "splice_insert: base has no video track to align the splice on",
                ))?;
        let (snapped_video_dts, video_split) = snap_to_preceding_sync(&base[video_idx], at_ticks)
            .ok_or(Error::InvalidInput(
            "splice_insert: base video track has no samples",
        ))?;
        let video_timescale = u128::from(base[video_idx].spec.timescale.get());
        let video_cut_dts = snapped_video_dts;

        let ref_timescale = base[video_idx].spec.timescale;
        let join_dts_ref = boundary_decode_time(&base[video_idx], video_split) as i64;
        let ad_video = &ad[mapping[video_idx]];
        let incoming_ref_start = ad_video
            .samples
            .first()
            .and_then(|s| s.dts)
            .unwrap_or(ad_video.start_decode_time as i64);

        let mut tracks = Vec::with_capacity(base.len());
        let mut points = Vec::new();
        for (i, bt) in base.iter().enumerate() {
            let adt = &ad[mapping[i]];
            let track_ts = bt.spec.timescale;
            let split_index = if i == video_idx {
                video_split
            } else {
                let track_timescale = u128::from(bt.spec.timescale.get());
                let scaled = u128::from(video_cut_dts) * track_timescale;
                let cut_in_track_ticks =
                    u64::try_from(scaled.div_ceil(video_timescale)).unwrap_or(u64::MAX);
                sample_index_at_absolute_dts(bt, cut_in_track_ticks)
            };
            let boundary_dts = boundary_decode_time(bt, split_index);
            let insert_shift = rescale_ticks(join_dts_ref, ref_timescale, track_ts)
                - rescale_ticks(incoming_ref_start, ref_timescale, track_ts);
            let ad_span: u64 = adt
                .samples
                .iter()
                .map(|s| u64::from(s.duration_ticks()))
                .sum();
            let resume_shift: i64 = match adt.samples.first().and_then(|s| s.dts) {
                Some(first) => first
                    .saturating_add(insert_shift)
                    .saturating_add(ad_span as i64)
                    .saturating_sub(boundary_dts as i64),
                None => ad_span as i64,
            };
            let insert_count = adt.samples.len();
            if insert_count > 0 {
                let presentation_time = match adt.samples.first().and_then(|s| s.pts) {
                    Some(p) => u64::try_from(p.saturating_add(insert_shift)).unwrap_or(0),
                    None => boundary_dts,
                };
                points.push(SplicePoint {
                    track_id: bt.spec.track_id,
                    sample_index: split_index,
                    presentation_time,
                });
            }
            if split_index < bt.samples.len() {
                let presentation_time = match bt.samples[split_index].pts {
                    Some(p) => u64::try_from(p.saturating_add(resume_shift)).unwrap_or(0),
                    None => boundary_dts.saturating_add(ad_span),
                };
                points.push(SplicePoint {
                    track_id: bt.spec.track_id,
                    sample_index: split_index + insert_count,
                    presentation_time,
                });
            }
            tracks.push(TrackSplice {
                track_id: bt.spec.track_id,
                split_index,
                insert_count,
                insert_track: adt.spec.track_id,
                insert_index: mapping[i],
                insert_shift,
                resume_shift,
            });
        }
        Ok(Self {
            tracks,
            movie_timescale,
            points,
        })
    }
}

/// Apply a splice to a per-track sample stream.
///
/// With a [`SplicePlan`] a sample is passed through unchanged before the
/// track's split, shifted by its insert shift inside the inserted run, and by
/// its resume shift after it. Without one ([`SpliceStage::switching`], Decision
/// 6) the stage holds at most one GOP per track, capped by the caller, and
/// [`SpliceStage::splice`] drops that held GOP so the switch lands on the
/// preceding sync sample. A track the plan does not name passes through
/// untouched.
#[derive(Debug, Clone)]
pub struct SpliceStage {
    plan: Option<SplicePlan>,
    counts: BTreeMap<TrackId, usize>,
    planless: Option<Planless>,
    ready: VecDeque<(TrackId, Sample)>,
}

/// The plan-less state: one held GOP per track, bounded in bytes.
#[derive(Debug, Clone)]
struct Planless {
    max_gop_bytes: u64,
    gops: BTreeMap<TrackId, Gop>,
}

/// The samples held since a track's last emitted GOP boundary.
#[derive(Debug, Clone, Default)]
struct Gop {
    held: VecDeque<Sample>,
    bytes: u64,
}

impl SpliceStage {
    /// Apply `plan` to the stream.
    #[must_use]
    pub fn new(plan: SplicePlan) -> Self {
        Self {
            plan: Some(plan),
            counts: BTreeMap::new(),
            planless: None,
            ready: VecDeque::new(),
        }
    }

    /// Apply a plan-less splice: hold at most one GOP per track, capped by
    /// `max_gop_bytes`, and snap a [`splice`](Self::splice) to the preceding
    /// sync sample.
    #[must_use]
    pub fn switching(max_gop_bytes: u64) -> Self {
        Self {
            plan: None,
            counts: BTreeMap::new(),
            planless: Some(Planless {
                max_gop_bytes,
                gops: BTreeMap::new(),
            }),
            ready: VecDeque::new(),
        }
    }

    /// The plan this stage applies, when it has one.
    #[must_use]
    pub fn plan(&self) -> Option<&SplicePlan> {
        self.plan.as_ref()
    }

    /// Switch source now: in plan-less mode the held GOP of every track is
    /// dropped, so the cut lands on the preceding sync sample (the last
    /// emitted sample ends where that sync sample begins).
    ///
    /// # Errors
    /// [`Error::InvalidInput`] when the stage was built with a plan, which
    /// already fixes the splice point.
    pub fn splice(&mut self) -> Result<()> {
        let Some(planless) = self.planless.as_mut() else {
            return Err(Error::InvalidInput(
                "splice: a plan already fixes the splice point",
            ));
        };
        for gop in planless.gops.values_mut() {
            gop.held.clear();
            gop.bytes = 0;
        }
        Ok(())
    }

    fn shift_for(&self, track: TrackId, index: usize) -> i64 {
        let Some(plan) = &self.plan else {
            return 0;
        };
        match plan.plan_for(track) {
            None => 0,
            Some(splice) if index < splice.split_index => 0,
            Some(splice) if index < splice.split_index.saturating_add(splice.insert_count) => {
                splice.insert_shift
            }
            Some(splice) => splice.resume_shift,
        }
    }

    fn feed_gop(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        let planless = self
            .planless
            .as_mut()
            .ok_or(Error::InvalidInput("splice: no plan and no GOP state"))?;
        let gop = planless.gops.entry(track).or_default();
        if sample.flags.is_sync && !gop.held.is_empty() {
            while let Some(done) = gop.held.pop_front() {
                self.ready.push_back((track, done));
            }
            gop.bytes = 0;
        }
        if gop.held.is_empty() && !sample.flags.is_sync {
            self.ready.push_back((track, sample));
            return Ok(());
        }
        let bytes = u64::try_from(sample.data.len()).unwrap_or(u64::MAX);
        if gop.bytes.saturating_add(bytes) > planless.max_gop_bytes {
            return Err(Error::SpliceGopOverflow {
                cap: planless.max_gop_bytes,
            });
        }
        gop.bytes = gop.bytes.saturating_add(bytes);
        gop.held.push_back(sample);
        Ok(())
    }
}

impl Stage for SpliceStage {
    type In<'a> = (TrackId, Sample);
    type Out = (TrackId, Sample);
    type Error = Error;

    fn feed(&mut self, (track, mut sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
        if self.plan.is_none() {
            return self.feed_gop(track, sample);
        }
        let index = self.counts.get(&track).copied().unwrap_or(0);
        let shift = self.shift_for(track, index);
        self.counts.insert(track, index.saturating_add(1));
        if shift != 0
            && let Some(timing) = sample.timing.as_mut()
        {
            timing.shift(shift);
        }
        self.ready.push_back((track, sample));
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        let Some(planless) = self.planless.as_mut() else {
            return Ok(());
        };
        let mut held = Vec::new();
        for (track, gop) in &mut planless.gops {
            while let Some(sample) = gop.held.pop_front() {
                held.push((*track, sample));
            }
            gop.bytes = 0;
        }
        for item in held {
            self.ready.push_back(item);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{SampleFlags, SampleTiming};
    use alloc::vec;
    use alloc::vec::Vec;

    const GOP_SAMPLES: usize = 4;
    const SAMPLE_BYTES: usize = 4;

    fn sample(index: usize, sync: bool) -> Sample {
        let dts = index as i64 * 100;
        Sample {
            data: vec![index as u8; SAMPLE_BYTES].into(),
            timing: Some(SampleTiming::timed_with_duration(dts, dts, 100)),
            flags: SampleFlags { is_sync: sync },
            provenance: None,
            display: None,
            protection: None,
        }
    }

    fn drain(stage: &mut SpliceStage) -> Vec<Sample> {
        let mut out = Vec::new();
        while let Some((_, sample)) = stage.poll() {
            out.push(sample);
        }
        out
    }

    #[test]
    fn a_planless_splice_snaps_to_the_preceding_sync_sample() {
        let track = TrackId::new(1);
        let mut stage = SpliceStage::switching(1 << 20);
        let mut out = Vec::new();
        for index in 0..10 {
            stage
                .feed(
                    (track, sample(index, index % GOP_SAMPLES == 0)),
                    Timestamp::ZERO,
                )
                .expect("feed");
            out.extend(drain(&mut stage));
        }
        // Two complete GOPs (0..8) are out; 8 and 9 are held.
        assert_eq!(out.len(), 8, "one GOP is held back");
        stage.splice().expect("a plan-less splice");
        stage.finish().expect("finish");
        out.extend(drain(&mut stage));
        assert_eq!(
            out.len(),
            8,
            "the splice drops the held GOP, so the cut lands on the preceding sync"
        );
        assert_eq!(
            out.iter()
                .map(|s| s.timing.and_then(|t| t.dts()))
                .collect::<Vec<_>>(),
            (0..8).map(|i| Some(i as i64 * 100)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_planless_gop_over_its_byte_cap_is_a_typed_error() {
        let track = TrackId::new(1);
        let cap = (SAMPLE_BYTES * 3) as u64;
        let mut stage = SpliceStage::switching(cap);
        let mut err = None;
        for index in 0..GOP_SAMPLES {
            if let Err(e) = stage.feed(
                (track, sample(index, index % GOP_SAMPLES == 0)),
                Timestamp::ZERO,
            ) {
                err = Some(e);
                break;
            }
        }
        assert_eq!(err, Some(Error::SpliceGopOverflow { cap }));
    }

    #[test]
    fn finish_flushes_a_held_gop_without_a_splice() {
        let track = TrackId::new(1);
        let mut stage = SpliceStage::switching(1 << 20);
        for index in 0..6 {
            stage
                .feed(
                    (track, sample(index, index % GOP_SAMPLES == 0)),
                    Timestamp::ZERO,
                )
                .expect("feed");
        }
        stage.finish().expect("finish");
        assert_eq!(
            drain(&mut stage).len(),
            6,
            "finish emits the held GOP so no sample is lost"
        );
    }
}
