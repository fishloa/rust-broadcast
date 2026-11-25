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
//! transform never holds a whole [`Media`]. The former whole-input
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

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, rescale_i64};

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::Sample;
use crate::media::{Media, Track};

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
fn track_end_decode_time(track: &Track) -> u64 {
    if let Some(last) = track.samples.last()
        && let Some(dts) = last.timing.and_then(|t| t.dts())
    {
        let dts = u64::try_from(dts).unwrap_or(0);
        return dts.saturating_add(last.timing.and_then(|t| t.duration()).unwrap_or(0) as u64);
    }
    let span: u64 = track
        .samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
        .sum();
    track.start_decode_time.saturating_add(span)
}

/// Match `a.tracks[i]` to an index in `b`: by `track_id` first, else by index.
/// The mapping is injective; mismatched counts, an unavailable match, or an
/// incompatible codec kind/timescale is [`Error::InvalidInput`].
fn match_tracks(a: &Media, b: &Media) -> Result<Vec<usize>> {
    if a.tracks.len() != b.tracks.len() {
        return Err(Error::InvalidInput(
            "splice: media have differing track counts",
        ));
    }
    let mut mapping = Vec::with_capacity(a.tracks.len());
    let mut used = alloc::vec![false; b.tracks.len()];
    for (i, at) in a.tracks.iter().enumerate() {
        let bj = b
            .tracks
            .iter()
            .enumerate()
            .find(|(j, bt)| !used[*j] && bt.spec.track_id == at.spec.track_id)
            .map(|(j, _)| j)
            .or_else(|| (!used[i]).then_some(i))
            .ok_or(Error::InvalidInput(
                "splice: no unclaimed matching track in b (non-injective match)",
            ))?;
        used[bj] = true;
        let bt = &b.tracks[bj];
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
fn pick_reference_track(a: &[Track], b: &[Track], mapping: &[usize]) -> Option<usize> {
    if let Some(i) = a.iter().position(|t| t.spec.config.is_video()) {
        return Some(i);
    }
    a.iter().enumerate().position(|(i, at)| {
        at.samples
            .iter()
            .any(|s| s.timing.and_then(|t| t.dts()).is_some())
            || b[mapping[i]]
                .samples
                .iter()
                .any(|s| s.timing.and_then(|t| t.dts()).is_some())
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
pub fn snap_to_preceding_sync(track: &Track, at_ticks: u64) -> Option<(u64, usize)> {
    if track.samples.is_empty() {
        return None;
    }
    let mut fallback_dts = track.start_decode_time;
    let mut best: (u64, usize) = (track.start_decode_time, 0);
    for (i, s) in track.samples.iter().enumerate() {
        let dts = s
            .timing
            .and_then(|t| t.dts())
            .map(|d| u64::try_from(d).unwrap_or(0))
            .unwrap_or(fallback_dts);
        if dts > at_ticks {
            break;
        }
        if s.flags.is_sync {
            best = (dts, i);
        }
        fallback_dts = dts.saturating_add(s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64);
    }
    Some(best)
}

/// The decode time of `track`'s natural continuation at `split_index`, in its
/// own media timescale: the boundary sample's absolute `dts` when it has one,
/// the track's end decode time for a past-the-end split, else the duration
/// sum from the track start.
fn boundary_decode_time(track: &Track, split_index: usize) -> u64 {
    match track
        .samples
        .get(split_index)
        .and_then(|s| s.timing.and_then(|t| t.dts()))
    {
        Some(dts) => u64::try_from(dts).unwrap_or(0),
        None if split_index >= track.samples.len() => track_end_decode_time(track),
        None => {
            track.start_decode_time
                + track.samples[..split_index]
                    .iter()
                    .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
                    .sum::<u64>()
        }
    }
}

/// First sample index of `track` whose **absolute** decode time is at or beyond
/// `cut_ticks`; clamps to the sample count for a past-the-end cut. Compared in
/// `i128` so a negative (rebased) absolute time is simply "before the cut".
fn sample_index_at_absolute_dts(track: &Track, cut_ticks: u64) -> usize {
    let mut acc = track.start_decode_time;
    let cut = i128::from(cut_ticks);
    for (i, s) in track.samples.iter().enumerate() {
        let absolute = match s.timing.and_then(|t| t.dts()) {
            Some(dts) => i128::from(dts),
            None => i128::from(acc),
        };
        if absolute >= cut {
            return i;
        }
        acc = acc.saturating_add(s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64);
    }
    track.samples.len()
}

impl SplicePlan {
    /// Append `b` after `a` on a shared, contiguous, monotonic decode timeline.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the track sets are incompatible, or if a
    /// matched `b` track's first sample is not a sync sample.
    pub fn concat(a: &Media, b: &Media) -> Result<Self> {
        let mapping = match_tracks(a, b)?;
        for &bj in &mapping {
            if let Some(first) = b.tracks[bj].samples.first()
                && !first.flags.is_sync
            {
                return Err(Error::InvalidInput(
                    "concat: appended track does not begin on a sync sample",
                ));
            }
        }
        let ref_shift = pick_reference_track(&a.tracks, &b.tracks, &mapping).map(|ri| {
            let ref_timescale = a.tracks[ri].spec.timescale;
            let join_dts_ref = track_end_decode_time(&a.tracks[ri]) as i64;
            let bt = &b.tracks[mapping[ri]];
            let incoming_ref_start = bt
                .samples
                .first()
                .and_then(|s| s.timing.and_then(|t| t.dts()))
                .unwrap_or(bt.start_decode_time as i64);
            (ref_timescale, join_dts_ref, incoming_ref_start)
        });

        let mut tracks = Vec::with_capacity(a.tracks.len());
        let mut points = Vec::new();
        for (i, at) in a.tracks.iter().enumerate() {
            let bt = &b.tracks[mapping[i]];
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
                let presentation_time = match bt
                    .samples
                    .first()
                    .and_then(|s| s.timing.and_then(|t| t.pts()))
                {
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
            movie_timescale: a.movie_timescale,
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
    pub fn insert(base: &Media, ad: &Media, at_ticks: u64) -> Result<Self> {
        let mapping = match_tracks(base, ad)?;
        for &aj in &mapping {
            if let Some(first) = ad.tracks[aj].samples.first()
                && !first.flags.is_sync
            {
                return Err(Error::InvalidInput(
                    "splice_insert: ad track does not begin on a sync sample",
                ));
            }
        }
        let video_idx = base
            .tracks
            .iter()
            .position(|t| t.spec.config.is_video())
            .ok_or(Error::InvalidInput(
                "splice_insert: base has no video track to align the splice on",
            ))?;
        let (snapped_video_dts, video_split) =
            snap_to_preceding_sync(&base.tracks[video_idx], at_ticks).ok_or(
                Error::InvalidInput("splice_insert: base video track has no samples"),
            )?;
        let video_timescale = u128::from(base.tracks[video_idx].spec.timescale.get());
        let video_cut_dts = snapped_video_dts;

        let ref_timescale = base.tracks[video_idx].spec.timescale;
        let join_dts_ref = boundary_decode_time(&base.tracks[video_idx], video_split) as i64;
        let ad_video = &ad.tracks[mapping[video_idx]];
        let incoming_ref_start = ad_video
            .samples
            .first()
            .and_then(|s| s.timing.and_then(|t| t.dts()))
            .unwrap_or(ad_video.start_decode_time as i64);

        let mut tracks = Vec::with_capacity(base.tracks.len());
        let mut points = Vec::new();
        for (i, bt) in base.tracks.iter().enumerate() {
            let adt = &ad.tracks[mapping[i]];
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
                .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
                .sum();
            let resume_shift: i64 = match adt
                .samples
                .first()
                .and_then(|s| s.timing.and_then(|t| t.dts()))
            {
                Some(first) => first
                    .saturating_add(insert_shift)
                    .saturating_add(ad_span as i64)
                    .saturating_sub(boundary_dts as i64),
                None => ad_span as i64,
            };
            let insert_count = adt.samples.len();
            if insert_count > 0 {
                let presentation_time = match adt
                    .samples
                    .first()
                    .and_then(|s| s.timing.and_then(|t| t.pts()))
                {
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
                let presentation_time = match bt.samples[split_index].timing.and_then(|t| t.pts()) {
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
            movie_timescale: base.movie_timescale,
            points,
        })
    }
}

/// Apply a [`SplicePlan`] to a per-track sample stream.
///
/// A sample is passed through unchanged before the track's split, shifted by
/// its insert shift inside the inserted run, and by its resume shift after it.
/// A track the plan does not name passes through untouched. The stage holds no
/// samples and emits one per feed.
#[derive(Debug, Clone)]
pub struct SpliceStage {
    plan: SplicePlan,
    counts: BTreeMap<TrackId, usize>,
    ready: VecDeque<(TrackId, Sample)>,
}

impl SpliceStage {
    /// Apply `plan` to the stream.
    #[must_use]
    pub fn new(plan: SplicePlan) -> Self {
        Self {
            plan,
            counts: BTreeMap::new(),
            ready: VecDeque::new(),
        }
    }

    /// The plan this stage applies.
    #[must_use]
    pub fn plan(&self) -> &SplicePlan {
        &self.plan
    }

    fn shift_for(&self, track: TrackId, index: usize) -> i64 {
        match self.plan.plan_for(track) {
            None => 0,
            Some(splice) if index < splice.split_index => 0,
            Some(splice) if index < splice.split_index.saturating_add(splice.insert_count) => {
                splice.insert_shift
            }
            Some(splice) => splice.resume_shift,
        }
    }
}

impl Stage for SpliceStage {
    type In<'a> = (TrackId, Sample);
    type Out = (TrackId, Sample);
    type Error = Error;

    fn feed(&mut self, (track, mut sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
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
        Ok(())
    }
}
