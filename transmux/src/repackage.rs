//! fMP4/CMAF repackaging — IR transforms + a driver (ISO/IEC 14496-12; CMAF
//! ISO/IEC 23000-19).
//!
//! `transmux` is an any-to-any container hub: [`Fmp4Demux`]
//! parses fragmented ISOBMFF/CMAF into the neutral [`Media`] IR, and
//! [`Segmenter`] / [`CmafMux`](crate::media::CmafMux) mux the IR back into CMAF.
//! *Repackaging* composes those two ends with pure transforms on the IR in the
//! middle — no new box parsing:
//!
//! - **track-select** — [`Media::select_tracks`] keeps a chosen subset of tracks,
//!   preserving their source order and [`TrackSpec`]s.
//! - **trim** — [`Media::trim`] keeps only the samples whose *presentation time*
//!   (composition time = decode time + `composition_offset`) falls in the
//!   half-open window `[start, end)`, expressed in the **movie timescale**
//!   ([`Media::movie_timescale`]). The first kept video sample is guaranteed to
//!   be a sync sample: the window's lower edge is snapped **back** to the
//!   preceding random-access point on the segmentation anchor track (see
//!   [`Media::trim`] for the exact rule), so the output opens on a keyframe as
//!   CMAF requires (ISO/IEC 23000-19 §7.3.2.3: a CMAF Track's first media
//!   sample must be an IDR/SAP); every other track then starts from that same
//!   snapped instant, so audio and video stay aligned. Window times are on one
//!   decode-time origin common to every track (the earliest first-sample
//!   `dts`), and the resegmented output keeps each track's offset from that
//!   origin in its first `tfdt`.
//! - **resegment** — feed the (optionally selected/trimmed) IR through
//!   [`Segmenter`] at a new target segment duration to produce fresh CMAF init +
//!   media segments cut on the anchor track's keyframes.
//!
//! [`Repackage`] is the convenience driver: it demuxes fMP4 bytes, applies an
//! optional track selection + trim window, and resegments at a target duration,
//! returning the CMAF init segment plus the media segments.
//!
//! `no_std` + `alloc`.

use alloc::vec::Vec;

use broadcast_common::Unpackage;

use crate::error::{Error, Result};
use crate::media::{Fmp4Demux, Media, TimelineOrigin, Track, relative_decode_times};
use crate::pipeline::TrackSpec;
use crate::segmenter::{Segmenter, choose_anchor};

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
/// [`Sample::dts`](crate::pipeline::Sample::dts) shifted by the origin —
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
/// Returned as `i64` because `composition_offset` is signed and may push an
/// early sample's presentation time slightly negative.
fn presentation_times(track: &Track, origin: &TimelineOrigin) -> Vec<i64> {
    relative_decode_times(track, origin)
        .into_iter()
        .zip(&track.samples)
        .map(|(dts, s)| dts + s.composition_offset() as i64)
        .collect()
}

/// Convert a window edge given in the movie timescale to a track's media
/// timescale, rounding down (`floor`) so the half-open `[start, end)` semantics
/// are preserved without dropping a boundary sample.
fn rescale_floor(ticks: u64, from_timescale: u32, to_timescale: u32) -> i64 {
    if from_timescale == 0 || from_timescale == to_timescale {
        return ticks as i64;
    }
    // (ticks * to) / from, in 128-bit to avoid overflow on large timescales.
    ((ticks as u128 * to_timescale as u128) / from_timescale as u128) as i64
}

/// Convert a signed tick count between timescales, rounding down.
fn rescale_signed_floor(ticks: i64, from_timescale: u32, to_timescale: u32) -> i64 {
    if from_timescale == 0 || from_timescale == to_timescale {
        return ticks;
    }
    (ticks as i128 * to_timescale as i128).div_euclid(from_timescale as i128) as i64
}

/// The index of the track whose next un-fed sample has the earliest decode
/// time, compared as exact rationals (`dts / timescale`).
///
/// The comparison cross-multiplies in `u128` (`a.dts * b.timescale` against
/// `b.dts * a.timescale`) rather than scaling every track onto a common LCM of
/// the timescales: the timescales come from untrusted `mdhd` boxes, and three
/// large coprime ones make that LCM exceed `u64`, so an `a / gcd * b` fold
/// panics in debug and wraps in release. A `u128` cross-multiply cannot
/// overflow for any pair of `u32` timescales. A timescale of 0 is treated as 1
/// (a track with no timescale still has to be placeable). Ties go to the
/// **lowest track index**, so the order is deterministic. Returns `None` when
/// every track's samples have been consumed.
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
        let lhs = dts[ti] * media.tracks[current].spec.timescale.max(1) as u128;
        let rhs = dts[current] * track.spec.timescale.max(1) as u128;
        if lhs < rhs {
            best = Some(ti);
        }
    }
    best
}

impl Media {
    /// Select a subset of tracks by their **index** (position in
    /// [`Media::tracks`]), preserving the given order.
    ///
    /// The chosen tracks are cloned with their [`TrackSpec`]s and samples
    /// intact; [`Media::movie_timescale`] is carried through unchanged.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `indices` is empty or names an out-of-range
    /// track index.
    pub fn select_tracks(&self, indices: &[usize]) -> Result<Media> {
        if indices.is_empty() {
            return Err(Error::InvalidInput("select_tracks: empty track selection"));
        }
        let mut tracks = Vec::with_capacity(indices.len());
        for &i in indices {
            let t = self.tracks.get(i).ok_or(Error::InvalidInput(
                "select_tracks: track index out of range",
            ))?;
            tracks.push(t.clone());
        }
        Ok(Media::new(tracks, self.movie_timescale))
    }

    /// Select a subset of tracks by predicate over each [`Track`], preserving
    /// source order.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the predicate keeps no track.
    pub fn select_tracks_by<F>(&self, mut keep: F) -> Result<Media>
    where
        F: FnMut(&Track) -> bool,
    {
        let tracks: Vec<Track> = self.tracks.iter().filter(|t| keep(t)).cloned().collect();
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "select_tracks_by: predicate kept no track",
            ));
        }
        Ok(Media::new(tracks, self.movie_timescale))
    }

    /// Trim to the half-open presentation-time window `[start, end)`, expressed
    /// in the **movie timescale** ([`Media::movie_timescale`]).
    ///
    /// For each track the window is rescaled into that track's media timescale
    /// and every sample whose composition (presentation) time — decode time +
    /// `composition_offset`, measured from the earliest first-sample decode
    /// time across all tracks — lies in `[start, end)` is kept.
    ///
    /// To satisfy the CMAF constraint that a track opens on a random-access
    /// point (ISO/IEC 23000-19 §7.3.2.3), the **anchor** track (`anchor_index`:
    /// first video track, else the first anchor-capable track) is back-trimmed:
    /// the first sample it keeps is the nearest sync sample at or before the
    /// first sample the raw window would select. Every other track's lower
    /// edge moves back to that sync sample's presentation time, so the tracks
    /// stay aligned (audio frames are all sync samples, so their first kept
    /// sample is already a random-access point); upper edges stay at `end`.
    ///
    /// The returned tracks keep their samples' absolute timestamps, with each
    /// [`Track::start_decode_time`] set to its first kept sample's `dts`.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `start >= end`, the media has no tracks, the
    /// window selects no sample on any track, or (only possible on an already
    /// pathological input) no track is anchor-capable.
    pub fn trim(&self, start: u64, end: u64) -> Result<Media> {
        if start >= end {
            return Err(Error::InvalidInput("trim: start must be < end"));
        }
        if self.tracks.is_empty() {
            return Err(Error::InvalidInput("trim: media has no tracks"));
        }
        let anchor = anchor_index(self)?;
        let origin = TimelineOrigin::of(&self.tracks);

        // The anchor first: its window start snaps back to a sync sample, and
        // every other track then starts from that same instant (issue #1021).
        let anchor_track = &self.tracks[anchor];
        let anchor_ts = anchor_track.spec.timescale;
        let anchor_pts = presentation_times(anchor_track, &origin);
        let anchor_lo = rescale_floor(start, self.movie_timescale, anchor_ts);
        let anchor_hi = rescale_floor(end, self.movie_timescale, anchor_ts);
        let snapped = anchor_pts
            .iter()
            .position(|&p| p >= anchor_lo && p < anchor_hi)
            .map(|mut idx| {
                while idx > 0 && !anchor_track.samples[idx].flags.is_sync {
                    idx -= 1;
                }
                idx
            });

        let mut out_tracks = Vec::with_capacity(self.tracks.len());
        let mut kept_any = false;
        for (ti, track) in self.tracks.iter().enumerate() {
            let ts = track.spec.timescale;
            let pts = presentation_times(track, &origin);
            let hi = rescale_floor(end, self.movie_timescale, ts);
            let start_idx = if ti == anchor {
                snapped
            } else {
                // Lower edge = the snapped anchor start, else the raw window.
                let lo = match snapped {
                    Some(idx) => rescale_signed_floor(anchor_pts[idx], anchor_ts, ts),
                    None => rescale_floor(start, self.movie_timescale, ts),
                };
                pts.iter().position(|&p| p >= lo && p < hi)
            };
            let mut kept = Vec::new();
            if let Some(start_idx) = start_idx {
                for (s, &p) in track.samples[start_idx..].iter().zip(&pts[start_idx..]) {
                    // Stop once presentation time reaches the window's upper edge.
                    if p >= hi {
                        break;
                    }
                    kept.push(s.clone());
                }
            }
            if !kept.is_empty() {
                kept_any = true;
            }
            let first_dts = kept.iter().find_map(|s| s.dts).unwrap_or(0).max(0) as u64;
            out_tracks.push(Track::new_at(track.spec.clone(), kept, first_dts));
        }
        if !kept_any {
            return Err(Error::InvalidInput(
                "trim: window selected no samples on any track",
            ));
        }
        Ok(Media::new(out_tracks, self.movie_timescale))
    }

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
            .map(|s| s.duration.unwrap_or(0) as u64)
            .sum();
        Some((ticks, t.spec.timescale))
    }
}

/// A repackaging driver: demux fragmented MP4 → optional track-select → optional
/// trim → resegment at a new target duration → CMAF segments.
///
/// Compose it fluently:
///
/// ```no_run
/// use transmux::Repackage;
/// # fn f(fmp4: &[u8]) -> transmux::Result<()> {
/// let out = Repackage::new(2.0)          // 2-second target segments
///     .select_tracks(&[0])               // keep only track 0 (video)
///     .trim(0, 90_000)                    // first second (movie timescale)
///     .run(fmp4)?;
/// let _init = out.init_segment;
/// let _media = out.media_segments;       // Vec<Vec<u8>>
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct Repackage {
    target_duration_secs: f64,
    select: Option<Vec<usize>>,
    trim: Option<(u64, u64)>,
}

impl Repackage {
    /// Create a repackager that resegments at `target_duration_secs` seconds,
    /// keeping all tracks and the full timeline unless [`Repackage::select_tracks`]
    /// / [`Repackage::trim`] are set.
    pub fn new(target_duration_secs: f64) -> Self {
        Self {
            target_duration_secs,
            select: None,
            trim: None,
        }
    }

    /// Restrict the output to the given track indices (positions in the demuxed
    /// [`Media::tracks`]), preserving their order.
    pub fn select_tracks(mut self, indices: &[usize]) -> Self {
        self.select = Some(indices.to_vec());
        self
    }

    /// Trim to the half-open presentation-time window `[start, end)` in the movie
    /// timescale (see [`Media::trim`]).
    pub fn trim(mut self, start: u64, end: u64) -> Self {
        self.trim = Some((start, end));
        self
    }

    /// Apply the configured transforms to an already-demuxed [`Media`] and
    /// resegment, returning the CMAF init + media segments.
    ///
    /// # Errors
    /// Propagates [`Media::select_tracks`] / [`Media::trim`] / [`Segmenter`]
    /// errors.
    pub fn run_media(&self, media: &Media) -> Result<RepackageOutput> {
        let mut work = media.clone();
        if let Some(indices) = &self.select {
            work = work.select_tracks(indices)?;
        }
        if let Some((start, end)) = self.trim {
            work = work.trim(start, end)?;
        }
        self.segment(&work)
    }

    /// Demux fragmented MP4 `fmp4` bytes into the IR, apply the configured
    /// transforms, and resegment.
    ///
    /// # Errors
    /// Propagates [`Fmp4Demux`] / transform / [`Segmenter`] errors.
    pub fn run(&self, fmp4: &[u8]) -> Result<RepackageOutput> {
        let media = Fmp4Demux::new().unpackage(fmp4)?;
        self.run_media(&media)
    }

    /// Push a [`Media`] through a fresh [`Segmenter`] at the target duration.
    fn segment(&self, media: &Media) -> Result<RepackageOutput> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("repackage: media has no tracks"));
        }
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let movie_timescale = media.movie_timescale;
        let mut seg = Segmenter::new(specs, movie_timescale, self.target_duration_secs)?;
        // Each track's first `tfdt` is its first decode time on the origin
        // common to all tracks, so a track starting later keeps that offset
        // (issue #1021) instead of every track restarting at 0.
        let origin = TimelineOrigin::of(&media.tracks);
        let mut dts = alloc::vec![0u128; media.tracks.len()];
        for (ti, track) in media.tracks.iter().enumerate() {
            let first = relative_decode_times(track, &origin)
                .first()
                .map_or(0, |&t| t.max(0) as u64);
            seg.set_start_decode_time(track.spec.track_id, first)?;
            dts[ti] = u128::from(first);
        }
        let init_segment = seg.init_segment()?;
        let mut seg_ready: Vec<Vec<u8>> = Vec::new();

        // Feed samples interleaved in global decode-time order (each track's
        // own timescale, normalised to a common rate). The [`Segmenter`] cuts on
        // the anchor's keyframe once the target is reached and emits *all*
        // tracks' buffered samples for that segment, so audio must arrive
        // alongside the video it is coincident with — feeding a whole track at a
        // time would strand all of one track in the final segment. The merge is
        // a k-way step over per-track cursors, picking the track whose next
        // sample has the earliest normalised decode time.
        let mut cursors = alloc::vec![0usize; media.tracks.len()];
        // The pick is [`earliest_decode_time_track`] — see its docs for why the
        // comparison is a cross-multiplied rational rather than an LCM scale.
        while let Some(ti) = earliest_decode_time_track(media, &dts, &cursors) {
            let track = &media.tracks[ti];
            let sample = &track.samples[cursors[ti]];
            seg.push(track.spec.track_id, sample.clone())?;
            dts[ti] += sample.duration.unwrap_or(0) as u128;
            cursors[ti] += 1;
            for s in seg.take_ready() {
                seg_ready.push(s);
            }
        }
        seg.flush()?;
        seg_ready.extend(seg.take_ready());
        Ok(RepackageOutput {
            init_segment,
            media_segments: seg_ready,
        })
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
    /// fragmented-MP4 byte stream — the form
    /// [`Fmp4Demux`] re-parses.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::CodecConfig;
    use crate::pipeline::Sample;
    use alloc::vec;

    /// A track with `count` samples one tick apart, at `timescale`.
    fn track(timescale: u32, count: usize) -> Track {
        let samples = (0..count)
            .map(|i| {
                let dts = i as i64;
                Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(1), true)
            })
            .collect();
        Track::new(
            TrackSpec::new(
                1,
                timescale,
                CodecConfig::Aac {
                    esds: crate::mp4esds::EsdsBox::new(crate::mp4esds::ESDescriptor::new(
                        1,
                        0,
                        Some(crate::mp4esds::DecoderConfigDescriptor::new(
                            0x40,
                            0x05,
                            false,
                            0,
                            0,
                            0,
                            Some(crate::mp4esds::DecoderSpecificInfo::new(vec![0x12, 0x10])),
                        )),
                        Some(crate::mp4esds::SLConfigDescriptor::predefined_two()),
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
        Media::new(timescales.iter().map(|&t| track(t, 4)).collect(), 1000)
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

    /// A zero timescale is treated as 1 Hz rather than dividing by zero or
    /// comparing against a zero denominator.
    #[test]
    fn zero_timescale_is_treated_as_one() {
        let m = media(&[0, 1]);
        // Track 0 at "1 Hz": 0 ticks = 0 s. Track 1 at 1 Hz: 0 ticks = 0 s.
        // A tie -> index 0. Then track 0 with 1 tick = 1 s vs track 1's 1 tick
        // = 1 s -> still a tie -> index 0.
        assert_eq!(pick(&m, &[0, 0], &[0, 0]), Some(0));
        assert_eq!(pick(&m, &[1, 1], &[0, 0]), Some(0));
        // And a real timescale can beat the zero one.
        let m = media(&[1, 1_000_000]);
        assert_eq!(pick(&m, &[1, 0], &[0, 0]), Some(1));
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
}
