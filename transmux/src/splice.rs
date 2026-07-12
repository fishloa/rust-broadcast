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
//! [`Sample`] timing is *relative*: each sample carries a `duration`, a sample's
//! decode time (DTS) is the running sum of preceding durations, and its
//! presentation time (PTS) is `DTS + composition_offset`. The only *absolute*
//! datum is [`Track::start_decode_time`] — the decode time of the track's first
//! sample — which [`CmafMux`](crate::media::CmafMux) writes as the fragment
//! `tfdt` `baseMediaDecodeTime` (ISO/IEC 14496-12:2015 §8.8.12). A track's
//! **end decode time** is therefore `start_decode_time + Σ durations`. The joins
//! here place the appended/inserted content so that its `start_decode_time`
//! exactly meets the preceding content's end decode time, keeping the muxed
//! `tfdt`s monotonic non-decreasing across the join. Coded sample bytes are
//! preserved byte-for-byte; only timing anchors/durations are recomputed.
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
/// time**): `start_decode_time + Σ sample durations`.
fn track_end_decode_time(track: &Track) -> u64 {
    let span: u64 = track.samples.iter().map(|s| s.duration as u64).sum();
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

/// Append `b` after `a` on a shared, contiguous, monotonic decode timeline.
///
/// Tracks are matched pairwise (by `track_id`, else by index). For each matched
/// track, `b`'s samples follow `a`'s with `b` rebased so its first sample's
/// decode time equals `a`'s **end decode time**
/// (`a.start_decode_time + Σ a.durations`) — i.e. contiguous, no gap or overlap.
/// Sample `data` is preserved byte-for-byte; `duration`/`is_sync`/
/// `composition_offset` are carried through unchanged. The movie timescale is
/// taken from `a`.
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
        if let Some(first) = b.tracks[bj].samples.first() {
            if !first.is_sync {
                return Err(Error::InvalidInput(
                    "concat: appended track does not begin on a sync sample",
                ));
            }
        }
    }

    let mut out_tracks = Vec::with_capacity(a.tracks.len());
    let mut points = Vec::new();
    for (i, at) in a.tracks.iter().enumerate() {
        let bt = &b.tracks[mapping[i]];
        let join_dts = track_end_decode_time(at);

        let mut samples = at.samples.clone();
        let join_index = samples.len();
        samples.extend(bt.samples.iter().cloned());

        // The join is a discontinuity only when `b` actually contributes samples.
        if !bt.samples.is_empty() {
            points.push(SplicePoint {
                track_id: at.spec.track_id,
                sample_index: join_index,
                // Presentation time of b's first sample on the joined timeline:
                // it decodes at join_dts (== a's end), plus its composition offset.
                presentation_time: join_dts
                    .saturating_add_signed(bt.samples[0].composition_offset as i64),
            });
        }

        out_tracks.push(Track::new_at(
            at.spec.clone(),
            samples,
            at.start_decode_time,
        ));
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
    let mut dts = track.start_decode_time;
    // The best (latest) sync sample seen at or before `at_ticks`. Seed with the
    // first sample so a request before the track start still snaps into range.
    let mut best: (u64, usize) = (track.start_decode_time, 0);
    for (i, s) in track.samples.iter().enumerate() {
        if dts > at_ticks {
            break;
        }
        if s.is_sync {
            best = (dts, i);
        }
        dts = dts.saturating_add(s.duration as u64);
    }
    Some(best)
}

/// Splice `ad` into `base` at `at_ticks` (server-side ad insertion).
///
/// The result plays `base` up to the splice boundary, then `ad` (rebased to
/// start at the boundary), then the remainder of `base` rebased forward by
/// `ad`'s duration — one monotonic decode timeline across both joins.
///
/// `at_ticks` is an absolute decode time in the base **video** track's media
/// timescale; it is **snapped to the nearest preceding sync sample** of that
/// track (a splice must land on a random-access point). The base's other tracks
/// (audio) are cut at the same *wall-clock* offset corresponding to that video
/// split — the video-timescale offset is rescaled into each other track's own
/// media timescale (audio is virtually never carried on the same timescale as
/// video, e.g. 90 kHz video vs. 44.1/48 kHz audio) before searching for its
/// split sample; audio is not independently RAP-aligned, but every audio sample
/// is itself a sync sample, so the audio cut is always on a RAP. Each track's
/// `ad` contribution is placed at the snapped boundary and the resumed base is
/// shifted by that track's own `ad` span, keeping every track's timeline
/// contiguous and monotonic. The snapped time is exposed on the returned
/// [`SplicePoint`]s.
///
/// # Errors
/// [`Error::InvalidInput`] if the track sets are incompatible (see [`concat`](fn@concat)),
/// if `base` has no video track to align on, or if a matched `ad` track's first
/// sample is not a sync sample.
pub fn splice_insert(base: &Media, ad: &Media, at_ticks: u64) -> Result<SpliceResult> {
    let mapping = match_tracks(base, ad)?;

    for &aj in &mapping {
        if let Some(first) = ad.tracks[aj].samples.first() {
            if !first.is_sync {
                return Err(Error::InvalidInput(
                    "splice_insert: ad track does not begin on a sync sample",
                ));
            }
        }
    }

    // Pick the base video track to align the splice on, and snap the request to
    // its preceding sync sample.
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

    let mut out_tracks = Vec::with_capacity(base.tracks.len());
    let mut points = Vec::new();

    for (i, bt) in base.tracks.iter().enumerate() {
        let adt = &ad.tracks[mapping[i]];

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

        let ad_span: u64 = adt.samples.iter().map(|s| s.duration as u64).sum();

        let mut samples: Vec<Sample> = Vec::with_capacity(bt.samples.len() + adt.samples.len());
        // 1. Base up to the split (unchanged).
        samples.extend(bt.samples[..split_index].iter().cloned());
        // 2. The ad.
        let ad_index = samples.len();
        samples.extend(adt.samples.iter().cloned());
        // 3. The remainder of the base (bytes unchanged; timing shifts because the
        //    ad's duration now sits before it on the shared relative timeline).
        let resume_index = samples.len();
        samples.extend(bt.samples[split_index..].iter().cloned());

        // The decode time of this track's split boundary on the base timeline.
        let boundary_dts: u64 = bt.start_decode_time
            + bt.samples[..split_index]
                .iter()
                .map(|s| s.duration as u64)
                .sum::<u64>();

        // Ad-in point.
        if !adt.samples.is_empty() {
            points.push(SplicePoint {
                track_id: bt.spec.track_id,
                sample_index: ad_index,
                presentation_time: boundary_dts
                    .saturating_add_signed(adt.samples[0].composition_offset as i64),
            });
        }
        // Resume point (the base sample that follows the ad), only if base has a
        // remainder after the split.
        if split_index < bt.samples.len() {
            let resume_dts = boundary_dts + ad_span;
            points.push(SplicePoint {
                track_id: bt.spec.track_id,
                sample_index: resume_index,
                presentation_time: resume_dts
                    .saturating_add_signed(bt.samples[split_index].composition_offset as i64),
            });
        }

        out_tracks.push(Track::new_at(
            bt.spec.clone(),
            samples,
            bt.start_decode_time,
        ));
    }

    Ok(SpliceResult {
        media: Media::new(out_tracks, base.movie_timescale),
        discontinuity_points: points,
    })
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
        acc = acc.saturating_add(s.duration as u64);
    }
    track.samples.len()
}
