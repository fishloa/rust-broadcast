//! IR-level timeline splice / concatenation → server-side ad insertion (SSAI),
//! operating on the [`Media`] IR before muxing (issue #475).
//!
//! Where [`crate::rebase`] conditions a *single* timeline (rebase-to-zero,
//! offset, 33-bit wrap-unroll, gap insertion), this module *joins* two
//! [`Media`] timelines into one monotonic decode timeline:
//!
//! - [`concat`](fn@concat) — append `b` after `a` on a shared timeline: for each matched
//!   track, `b`'s samples follow `a`'s, with `b` rebased so its first sample's
//!   decode time equals `a`'s end decode time (contiguous, no gap or overlap).
//! - [`splice_insert`] — SSAI: play `base` up to a splice time, insert `ad`,
//!   then resume the remainder of `base` shifted forward by `ad`'s duration.
//!
//! # Timeline model (ISO/IEC 14496-12 `tfdt`)
//!
//! [`Sample`] timing is *absolute* (media plane step 2c): each sample carries
//! its own optional decode time (DTS) and presentation time (PTS =
//! DTS + composition_offset), in the track's media timescale.
//! [`Track::start_decode_time`] — which [`CmafMux`](crate::media::CmafMux)
//! writes as the *first* fragment's `tfdt` `baseMediaDecodeTime` (ISO/IEC
//! 14496-12:2015 §8.8.12) — is the track's start anchor, not a running cursor:
//! a source that re-seeds `tfdt` per fragment (a discontinuous or gapped
//! capture) can leave a real gap between it and a later sample's true `dts`.
//! This module therefore reads each join/snap boundary's absolute `dts`
//! directly rather than reconstructing it as `start_decode_time + Σ
//! durations`, falling back to that reconstruction only where a sample
//! genuinely carries no timestamp (a section-carried sample — SCTE-35/
//! DSM-CC/private sections — which has none to read). Coded sample bytes are
//! preserved byte-for-byte; only timing anchors/durations are recomputed.
//!
//! # Rebasing the spliced-in content (issue #782)
//!
//! Two independently-demuxed assets have unrelated absolute timelines (each
//! anchored on its own `tfdt`/PCR/FLV clock) — simply appending a second
//! [`Media`]'s samples after a join point, keeping their own file's absolute
//! `dts`/`pts`, produces an arbitrary jump instead of a contiguous join. So
//! [`concat`](fn@concat) and [`splice_insert`] shift every incoming sample's
//! `dts`/`pts` (when `Some`) by `join_dts - incoming_reference_start` before
//! placing it:
//!
//! - **One offset per splice, derived from a single reference track** — the
//!   video track if either side has one, else the first track that carries a
//!   real timestamp (`pick_reference_track`) — and applied uniformly to
//!   every matched track, converted into each track's own timescale
//!   (`rescale_ticks`). Deriving the shift independently per track instead
//!   would silently re-align tracks relative to each other and destroy A/V
//!   sync (a track's own boundary sample needn't correspond, wall-clock, to
//!   another track's).
//! - **Composition offsets survive exactly**: `dts` and `pts` shift by the
//!   same amount (`shift_samples`), so `pts - dts` is unchanged.
//! - **No accumulated rounding across repeated splices**: every splice call
//!   rescales the *fresh, already-materialized absolute* reference-track
//!   ticks (the join point and the incoming asset's own start) directly into
//!   each track's timescale — it never caches and re-rescales an
//!   already-rounded delta from a previous splice. A chain of splices reads
//!   real integers out of the previous call's result every time, so any
//!   sub-tick rounding from one splice can never compound into the next.
//! - **`dts: None` samples are left untouched** — a section-carried sample
//!   (SCTE-35/DSM-CC/private sections) has no timestamp to rebase and none is
//!   fabricated.
//!
//! The reference track's own contribution lands exactly on the join (no
//! rescale needed, same timescale); a non-reference track's contribution
//! lands within a fraction of a tick of the wall-clock join, preserving
//! whatever real inter-track relationship the incoming asset already had
//! (e.g. a small audio pre-roll ahead of video) rather than forcing every
//! track to butt exactly against its own predecessor.
//!
//! # Keyframe / RAP alignment
//!
//! A splice boundary must fall on a sync sample (random-access point) so the
//! spliced-in content — and the resumed base — can be decoded from the cut.
//! [`concat`](fn@concat) and [`splice_insert`] therefore require the inserted content's
//! first sample to be a sync sample ([`Error::InvalidInput`] otherwise). For
//! [`splice_insert`] the requested splice time is **snapped to the nearest
//! preceding sync sample** of the base's video track via [`snap_to_preceding_sync`]
//! (a helper, so the snap is independently testable); the snapped time is exposed
//! on the returned [`SplicePoint`]s.
//!
//! # Discontinuity signalling
//!
//! Each join is a media-timeline discontinuity (RFC 8216 §4.3.4.3). The result
//! is a [`SpliceResult`] carrying the [`Media`] plus the [`SplicePoint`]s (track
//! id + sample index + presentation time of each join), so a downstream HLS
//! packager / [`Segmenter`](crate::segmenter::Segmenter) can emit
//! `#EXT-X-DISCONTINUITY` before exactly those segments (drive the segmenter's
//! [`mark_discontinuity`](crate::segmenter::Segmenter::mark_discontinuity) at the
//! reported sample indices).
//!
//! # Follow-up
//!
//! Selecting splice *points* from SCTE-35 cue messages (parsing
//! `splice_info_section` `splice_time`/`break_duration` to decide *where* and how
//! long) is a follow-up — it would add a `scte35-splice` (+ `timed-metadata` for
//! PTS wrap handling) dependency. This module does the timeline mechanics for an
//! explicitly supplied time / point; a later transform can compute those points
//! from SCTE-35 and feed them here.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::pipeline::{CodecConfig, Sample};

/// A splice / concatenation join point in a resulting [`Media`] track.
///
/// Identifies the first sample of the spliced-in contribution so a downstream
/// packager can mark the containing segment discontinuous
/// (`#EXT-X-DISCONTINUITY`, RFC 8216 §4.3.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SplicePoint {
    /// The track this join occurs on.
    pub track_id: u32,
    /// Index, within the resulting track's `samples`, of the first sample of the
    /// spliced-in contribution (the sample that opens the discontinuous region).
    pub sample_index: usize,
    /// Presentation time (PTS = DTS + composition_offset) of that sample, in the
    /// track's media timescale ([`Track::timescale`](crate::media::Track::timescale)).
    pub presentation_time: u64,
}

/// The result of a splice / concatenation: the joined [`Media`] plus the join
/// points that should be signalled as discontinuities.
///
/// `discontinuity_points` lists one [`SplicePoint`] per matched track per join
/// (a [`concat`](fn@concat) yields one join, a [`splice_insert`] yields two — the ad-in and
/// the resume), in track then timeline order.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SpliceResult {
    /// The joined media on a single monotonic timeline.
    pub media: Media,
    /// The join points to signal downstream as `#EXT-X-DISCONTINUITY`.
    pub discontinuity_points: Vec<SplicePoint>,
}

/// A stable codec-kind token used only to check two tracks are the *same* codec
/// before joining their samples (dimensions/bitrate may differ across the join,
/// but the codec family and hence the sample entry must match).
fn codec_kind(config: &CodecConfig) -> &'static str {
    match config {
        CodecConfig::Avc { .. } => "avc",
        CodecConfig::Hevc { .. } => "hevc",
        CodecConfig::Vvc { .. } => "vvc",
        CodecConfig::Av1 { .. } => "av1",
        CodecConfig::Vp9 { .. } => "vp9",
        CodecConfig::Vp8 { .. } => "vp8",
        CodecConfig::Mpeg2Video { .. } => "mpeg2video",
        CodecConfig::Aac { .. } => "aac",
        CodecConfig::Ac3 { .. } => "ac3",
        CodecConfig::Eac3 { .. } => "eac3",
        CodecConfig::Ac4 { .. } => "ac4",
        CodecConfig::Opus { .. } => "opus",
        CodecConfig::Flac { .. } => "flac",
        CodecConfig::Dts { .. } => "dts",
        CodecConfig::MpegH { .. } => "mpegh",
        CodecConfig::MpegAudio { .. } => "mpegaudio",
        CodecConfig::Vorbis { .. } => "vorbis",
        CodecConfig::Data { .. } => "data",
        CodecConfig::Subtitle { .. } => "subtitle",
    }
}

/// Whether `config` is a video codec (used to pick the splice-alignment track).
fn is_video(config: &CodecConfig) -> bool {
    matches!(
        codec_kind(config),
        "avc" | "hevc" | "vvc" | "av1" | "vp9" | "vp8" | "mpeg2video"
    )
}

/// The decode time of a track's first sample beyond its last (its **end decode
/// time**).
///
/// Reads the last sample's own absolute `dts` (media plane step 2c) plus its
/// `duration`, when both are known — this is the authoritative value and can
/// legitimately diverge from `start_decode_time + Σ durations` after a
/// per-fragment `tfdt` reseed (`start_decode_time` only ever records the
/// *first* fragment's anchor, see `media.rs::absorb_fragment`), which is
/// exactly the desync this function used to reintroduce one step downstream
/// of that fix. Falls back to the duration-sum from `start_decode_time` only
/// when the last sample genuinely carries no timestamp at all (a
/// section-carried sample — SCTE-35/DSM-CC/private sections — which has no
/// absolute `dts` to read).
fn track_end_decode_time(track: &Track) -> u64 {
    if let Some(last) = track.samples.last()
        && let Some(dts) = last.dts
    {
        let dts = u64::try_from(dts).unwrap_or(0);
        return dts.saturating_add(last.duration.unwrap_or(0) as u64);
    }
    let span: u64 = track
        .samples
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum();
    track.start_decode_time.saturating_add(span)
}

/// Match `a.tracks[i]` to a track in `b`: by `track_id` first, else by index.
///
/// Returns, for each track in `a`, the index of its counterpart in `b`. Errors
/// if the track sets are incompatible (differing counts, an unmatched id, or a
/// matched pair whose codec kind or timescale differs).
fn match_tracks(a: &Media, b: &Media) -> Result<Vec<usize>> {
    if a.tracks.len() != b.tracks.len() {
        return Err(Error::InvalidInput(
            "splice: media have differing track counts",
        ));
    }
    let mut mapping = Vec::with_capacity(a.tracks.len());
    for (i, at) in a.tracks.iter().enumerate() {
        // Prefer an id match; fall back to the same positional index.
        let bj = b
            .tracks
            .iter()
            .position(|bt| bt.spec.track_id == at.spec.track_id)
            .unwrap_or(i);
        let bt = &b.tracks[bj];
        if codec_kind(&at.spec.config) != codec_kind(&bt.spec.config) {
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

/// Pick the single track (by index into `a`/`b`, matched via `mapping`) whose
/// timeline anchors the splice offset applied uniformly to every track
/// (issue #782): the video track if either side has one, else the first
/// track that carries at least one real timestamp on either side. `None`
/// when no track anywhere carries a timestamp — there is nothing to align
/// on, so the caller leaves every sample untouched.
///
/// Deliberately a *single* pick, not one per track: computing the offset
/// independently per track would silently re-align tracks relative to each
/// other and destroy A/V sync (see the module-level doc).
fn pick_reference_track(a: &[Track], b: &[Track], mapping: &[usize]) -> Option<usize> {
    if let Some(i) = a.iter().position(|t| is_video(&t.spec.config)) {
        return Some(i);
    }
    a.iter().enumerate().position(|(i, at)| {
        at.samples.iter().any(|s| s.dts.is_some())
            || b[mapping[i]].samples.iter().any(|s| s.dts.is_some())
    })
}

/// Rescale an absolute tick value from one track's media timescale into
/// another's, in 128-bit arithmetic to avoid overflow. Truncates toward zero
/// on a non-exact ratio (matching `repackage::rescale_floor`/`ts_mux::rescale`'s
/// existing technique elsewhere in this crate) — timescales are always
/// positive on a real track; an identity is returned for a degenerate zero or
/// an already-equal pair, with no arithmetic at all.
///
/// Used to convert the *single* splice offset derived from one reference
/// track ([`pick_reference_track`]) into every other matched track's own
/// timescale, so mixed-timescale tracks (e.g. 48 kHz audio alongside 90 kHz
/// video) shift by the same real-world amount of time. Callers always feed
/// this the fresh absolute tick values read directly off the current
/// samples — never an already-rescaled delta carried over from a previous
/// splice — which is what keeps sub-tick rounding from compounding across a
/// chain of splices (see the module-level doc).
fn rescale_ticks(ticks: i64, from_timescale: u32, to_timescale: u32) -> i64 {
    if from_timescale == 0 || to_timescale == 0 || from_timescale == to_timescale {
        return ticks;
    }
    ((ticks as i128 * to_timescale as i128) / from_timescale as i128) as i64
}

/// Shift every sample's absolute `dts`/`pts` — when present — by `offset`
/// ticks in the track's own media timescale.
///
/// Both fields move by the same amount, so each sample's composition offset
/// (`pts - dts`) is preserved exactly. A sample with `dts: None` (a
/// section-carried sample — SCTE-35/DSM-CC/private sections) has no
/// timestamp to rebase and is left untouched — never fabricated.
fn shift_samples(samples: &[Sample], offset: i64) -> Vec<Sample> {
    samples
        .iter()
        .cloned()
        .map(|mut s| {
            if let Some(d) = s.dts {
                s.dts = Some(d + offset);
            }
            if let Some(p) = s.pts {
                s.pts = Some(p + offset);
            }
            s
        })
        .collect()
}

/// The [`Track::start_decode_time`] anchor for a splice/concat result: the
/// first sample's own absolute `dts` when there is one — keeping the anchor
/// in lockstep with the sample it names — else `fallback` (the original
/// track's own anchor, used only when the result has no sample to read one
/// from, or whose first sample is untimed).
fn result_start_decode_time(samples: &[Sample], fallback: u64) -> u64 {
    samples
        .first()
        .and_then(|s| s.dts)
        .map(|d| u64::try_from(d).unwrap_or(0))
        .unwrap_or(fallback)
}

/// Append `b` after `a` on a shared, contiguous, monotonic decode timeline.
///
/// Tracks are matched pairwise (by `track_id`, else by index). For each
/// matched track, `b`'s samples follow `a`'s, rebased onto the join by the
/// single reference-track offset described at the module level (issue #782)
/// — `b`'s reference track lands exactly on `a`'s **end decode time**
/// (`a`'s last sample's own absolute `dts` + duration); every other matched
/// track shifts by the same offset, converted into its own timescale.
/// Sample `data` is preserved byte-for-byte; only `dts`/`pts` move, and only
/// when they were `Some` to begin with. The movie timescale is taken from
/// `a`.
///
/// The first sample of each `b` contribution is the splice point and is reported
/// in [`SpliceResult::discontinuity_points`].
///
/// # Errors
/// [`Error::InvalidInput`] if the track sets are incompatible (differing counts,
/// an unmatched id, or a matched pair whose codec kind or timescale differs), or
/// if a matched `b` track's first sample is not a sync sample (a splice boundary
/// must be a random-access point).
pub fn concat(a: &Media, b: &Media) -> Result<SpliceResult> {
    let mapping = match_tracks(a, b)?;

    // Every `b` track that carries samples must open on a sync sample.
    for &bj in &mapping {
        if let Some(first) = b.tracks[bj].samples.first()
            && !first.flags.is_sync
        {
            return Err(Error::InvalidInput(
                "concat: appended track does not begin on a sync sample",
            ));
        }
    }

    // The single reference-track shift (issue #782): derived once, here,
    // from fresh absolute ticks (`a`'s reference track's real end decode
    // time and `b`'s reference track's real start) — then converted into
    // each matched track's own timescale inside the loop below. Never
    // derived independently per track.
    let ref_shift = pick_reference_track(&a.tracks, &b.tracks, &mapping).map(|ri| {
        let ref_timescale = a.tracks[ri].spec.timescale;
        let join_dts_ref = track_end_decode_time(&a.tracks[ri]) as i64;
        let bt = &b.tracks[mapping[ri]];
        let incoming_ref_start = bt
            .samples
            .first()
            .and_then(|s| s.dts)
            .unwrap_or(bt.start_decode_time as i64);
        (ref_timescale, join_dts_ref, incoming_ref_start)
    });

    let mut out_tracks = Vec::with_capacity(a.tracks.len());
    let mut points = Vec::new();
    for (i, at) in a.tracks.iter().enumerate() {
        let bt = &b.tracks[mapping[i]];

        // Rescale the single reference offset into THIS track's own
        // timescale (rescaling the fresh absolute endpoints, not a cached
        // delta — see `rescale_ticks`'s doc), then shift b's samples by it.
        let shifted = match ref_shift {
            Some((ref_ts, join_dts_ref, incoming_ref_start)) => {
                let track_ts = at.spec.timescale;
                let offset = rescale_ticks(join_dts_ref, ref_ts, track_ts)
                    - rescale_ticks(incoming_ref_start, ref_ts, track_ts);
                shift_samples(&bt.samples, offset)
            }
            None => bt.samples.clone(),
        };

        let join_index = at.samples.len();
        let mut samples = at.samples.clone();
        samples.extend(shifted.iter().cloned());

        // The join is a discontinuity only when `b` actually contributes samples.
        if !shifted.is_empty() {
            // Presentation time of b's first (now-rebased) sample: its own
            // `pts` when it has one; a section-carried leading sample (no
            // timestamp at all) falls back to `a`'s reference-track end, the
            // best information available.
            let presentation_time = match shifted[0].pts {
                Some(p) => u64::try_from(p).unwrap_or(0),
                None => track_end_decode_time(at),
            };
            points.push(SplicePoint {
                track_id: at.spec.track_id,
                sample_index: join_index,
                presentation_time,
            });
        }

        let track_start = result_start_decode_time(&samples, at.start_decode_time);
        out_tracks.push(Track::new_at(at.spec.clone(), samples, track_start));
    }

    Ok(SpliceResult {
        media: Media::new(out_tracks, a.movie_timescale),
        discontinuity_points: points,
    })
}

/// Snap a requested decode time to the nearest **preceding** sync sample of a
/// track, returning `(snapped_decode_time, sample_index)`.
///
/// A splice boundary must land on a random-access point, so a requested
/// `at_ticks` that falls inside a GOP is pulled back to the decode time of the
/// most recent sync sample at or before it. If `at_ticks` precedes the first
/// sample, it snaps to the first sample (index 0). `at_ticks` is an absolute
/// decode time in `track`'s media timescale (the same units as
/// [`Track::start_decode_time`]).
///
/// Returns `None` when the track has no samples (nothing to snap to).
pub fn snap_to_preceding_sync(track: &Track, at_ticks: u64) -> Option<(u64, usize)> {
    if track.samples.is_empty() {
        return None;
    }
    // Running fallback decode time — advanced by duration only for a sample
    // with no absolute `dts` (a section-carried sample). Seed with
    // `start_decode_time` so a request before the track start still snaps
    // into range even on such a track.
    let mut fallback_dts = track.start_decode_time;
    let mut best: (u64, usize) = (track.start_decode_time, 0);
    for (i, s) in track.samples.iter().enumerate() {
        // Prefer the sample's own absolute `dts` (media plane step 2c) — the
        // authoritative decode time, which can legitimately diverge from
        // `start_decode_time + Σ durations` after a per-fragment `tfdt`
        // reseed (`media.rs::absorb_fragment`). Only fall back to the
        // running duration-sum for a genuinely timestamp-less
        // (section-carried) sample, which carries no absolute `dts` at all.
        let dts = s
            .dts
            .map(|d| u64::try_from(d).unwrap_or(0))
            .unwrap_or(fallback_dts);
        if dts > at_ticks {
            break;
        }
        if s.flags.is_sync {
            best = (dts, i);
        }
        fallback_dts = dts.saturating_add(s.duration.unwrap_or(0) as u64);
    }
    Some(best)
}

/// Splice `ad` into `base` at `at_ticks` (server-side ad insertion).
///
/// The result plays `base` up to the splice boundary, then `ad` — rebased
/// from its own file's independent timeline onto the boundary via the single
/// reference-track offset described at the module level (issue #782) — then
/// the remainder of `base`, itself rebased forward so it resumes exactly
/// where the rebased `ad` ends on each track — one monotonic decode timeline
/// across both joins.
///
/// `at_ticks` is an absolute decode time in the base **video** track's media
/// timescale; it is **snapped to the nearest preceding sync sample** of that
/// track (a splice must land on a random-access point). The base's other tracks
/// (audio) are cut at the same *wall-clock* offset corresponding to that video
/// split — the video-timescale offset is rescaled into each other track's own
/// media timescale (audio is virtually never carried on the same timescale as
/// video, e.g. 90 kHz video vs. 44.1/48 kHz audio) before searching for its
/// split sample; audio is not independently RAP-aligned, but every audio sample
/// is itself a sync sample, so the audio cut is always on a RAP. The snapped
/// time is exposed on the returned [`SplicePoint`]s.
///
/// # Errors
/// [`Error::InvalidInput`] if the track sets are incompatible (see [`concat`](fn@concat)),
/// if `base` has no video track to align on, or if a matched `ad` track's first
/// sample is not a sync sample.
pub fn splice_insert(base: &Media, ad: &Media, at_ticks: u64) -> Result<SpliceResult> {
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

    // Pick the base video track to align the splice on, and snap the request to
    // its preceding sync sample. This is also the reference track for the
    // ad-in rebase (issue #782): `splice_insert` always requires one, so
    // there is never an "else the first timed track" case to consider here
    // (unlike `concat`'s `pick_reference_track`).
    let video_idx = base
        .tracks
        .iter()
        .position(|t| is_video(&t.spec.config))
        .ok_or(Error::InvalidInput(
            "splice_insert: base has no video track to align the splice on",
        ))?;
    let (snapped_video_dts, video_split) =
        snap_to_preceding_sync(&base.tracks[video_idx], at_ticks).ok_or(Error::InvalidInput(
            "splice_insert: base video track has no samples",
        ))?;
    // Fraction of the video track (by decode time) at which the split falls,
    // used to place the same wall-clock cut on the other (audio) tracks. This
    // is in the *video* track's own timescale; §sample_index_at_offset below
    // rescales it into each other track's timescale before use.
    let split_offset_ticks =
        snapped_video_dts.saturating_sub(base.tracks[video_idx].start_decode_time);
    let video_timescale = u128::from(base.tracks[video_idx].spec.timescale.max(1));

    // The single reference-track shift for the ad-in point (issue #782):
    // derived once, here, from fresh absolute ticks — the base video
    // track's own real boundary decode time and the ad's video track's own
    // real start — then converted into every matched track's own timescale
    // inside the loop below, mirroring `concat`. Never derived
    // independently per track.
    let ref_timescale = base.tracks[video_idx].spec.timescale;
    let join_dts_ref = boundary_decode_time(&base.tracks[video_idx], video_split) as i64;
    let ad_video = &ad.tracks[mapping[video_idx]];
    let incoming_ref_start = ad_video
        .samples
        .first()
        .and_then(|s| s.dts)
        .unwrap_or(ad_video.start_decode_time as i64);

    let mut out_tracks = Vec::with_capacity(base.tracks.len());
    let mut points = Vec::new();

    for (i, bt) in base.tracks.iter().enumerate() {
        let adt = &ad.tracks[mapping[i]];
        let track_ts = bt.spec.timescale;

        // Where to cut this base track. For the video track it is the snapped
        // sync sample; for the others, the first sample whose decode time is at
        // or beyond the same *wall-clock* offset from the track start, rescaled
        // from the video track's timescale into this track's own (audio is
        // virtually never carried on the video track's timescale, e.g. 90 kHz
        // video vs. 44.1/48 kHz audio) — audio samples are all sync samples, so
        // this is always a valid RAP cut.
        let split_index = if i == video_idx {
            video_split
        } else {
            let track_timescale = u128::from(bt.spec.timescale.max(1));
            let offset_in_track_ticks =
                (u128::from(split_offset_ticks) * track_timescale / video_timescale) as u64;
            sample_index_at_offset(bt, offset_in_track_ticks)
        };

        // This track's own natural continuation point at the split, on the
        // ORIGINAL (pre-splice) base timeline — see `boundary_decode_time`.
        let boundary_dts = boundary_decode_time(bt, split_index);

        // Ad-in shift: the single reference offset (derived above from the
        // video track), rescaled into this track's own timescale — never
        // computed independently from this track's own boundary (that would
        // re-derive a per-track alignment and destroy A/V sync).
        let ad_offset = rescale_ticks(join_dts_ref, ref_timescale, track_ts)
            - rescale_ticks(incoming_ref_start, ref_timescale, track_ts);
        let shifted_ad = shift_samples(&adt.samples, ad_offset);

        let ad_span: u64 = adt
            .samples
            .iter()
            .map(|s| s.duration.unwrap_or(0) as u64)
            .sum();

        // Resume shift: push the remainder of base forward so — on THIS
        // track — it picks up exactly where the rebased ad content ends
        // (`shifted_ad`'s first real dts + this track's own ad span), rather
        // than at its original, now ad-occupied, position. A section-carried
        // ad contribution with no timestamp on this track at all has no
        // "where the ad ends" to read; falls back to the span alone (there
        // is nothing else to derive it from).
        let resume_shift: i64 = match shifted_ad.first().and_then(|s| s.dts) {
            Some(new_ad_first_dts) => new_ad_first_dts + ad_span as i64 - boundary_dts as i64,
            None => ad_span as i64,
        };
        let shifted_resume = shift_samples(&bt.samples[split_index..], resume_shift);

        let mut samples: Vec<Sample> = Vec::with_capacity(bt.samples.len() + adt.samples.len());
        // 1. Base up to the split (unchanged).
        samples.extend(bt.samples[..split_index].iter().cloned());
        // 2. The ad, rebased onto the boundary.
        let ad_index = samples.len();
        samples.extend(shifted_ad.iter().cloned());
        // 3. The remainder of the base, rebased forward so it resumes
        //    exactly where the rebased ad ends.
        let resume_index = samples.len();
        samples.extend(shifted_resume.iter().cloned());

        // Ad-in point.
        if !shifted_ad.is_empty() {
            let presentation_time = match shifted_ad[0].pts {
                Some(p) => u64::try_from(p).unwrap_or(0),
                None => boundary_dts,
            };
            points.push(SplicePoint {
                track_id: bt.spec.track_id,
                sample_index: ad_index,
                presentation_time,
            });
        }
        // Resume point (the base sample that follows the ad), only if base has a
        // remainder after the split.
        if split_index < bt.samples.len() {
            let presentation_time = match shifted_resume.first().and_then(|s| s.pts) {
                Some(p) => u64::try_from(p).unwrap_or(0),
                None => boundary_dts.saturating_add(ad_span),
            };
            points.push(SplicePoint {
                track_id: bt.spec.track_id,
                sample_index: resume_index,
                presentation_time,
            });
        }

        let track_start = result_start_decode_time(&samples, bt.start_decode_time);
        out_tracks.push(Track::new_at(bt.spec.clone(), samples, track_start));
    }

    Ok(SpliceResult {
        media: Media::new(out_tracks, base.movie_timescale),
        discontinuity_points: points,
    })
}

/// The decode time of `track`'s natural continuation at `split_index`, in
/// its own media timescale: the boundary sample's own absolute `dts` (media
/// plane step 2c) when there is one — authoritative, and can legitimately
/// diverge from `start_decode_time + Σ durations` after a per-fragment
/// `tfdt` reseed, exactly like `track_end_decode_time`/
/// `snap_to_preceding_sync` above. A past-the-end split (no remaining
/// sample) has no boundary sample to read, so it uses the track's own end
/// decode time instead; a section-carried boundary sample with no absolute
/// `dts` falls back to the duration-sum, same as everywhere else in this
/// module.
fn boundary_decode_time(track: &Track, split_index: usize) -> u64 {
    match track.samples.get(split_index).and_then(|s| s.dts) {
        Some(dts) => u64::try_from(dts).unwrap_or(0),
        None if split_index >= track.samples.len() => track_end_decode_time(track),
        None => {
            track.start_decode_time
                + track.samples[..split_index]
                    .iter()
                    .map(|s| s.duration.unwrap_or(0) as u64)
                    .sum::<u64>()
        }
    }
}

/// First sample index of `track` whose decode time (relative to the track start)
/// is at or beyond `offset_ticks`; clamps to the sample count for a past-the-end
/// offset.
fn sample_index_at_offset(track: &Track, offset_ticks: u64) -> usize {
    let mut acc = 0u64;
    for (i, s) in track.samples.iter().enumerate() {
        if acc >= offset_ticks {
            return i;
        }
        acc = acc.saturating_add(s.duration.unwrap_or(0) as u64);
    }
    track.samples.len()
}
