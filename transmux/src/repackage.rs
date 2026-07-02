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
//!   sample must be an IDR/SAP). Output decode times are re-based to zero: the
//!   IR carries only the retained samples and the muxers emit
//!   `base_media_decode_time = 0` for the first segment.
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
use crate::media::{Fmp4Demux, Media, Track};
use crate::pipeline::{CodecConfig, TrackSpec};
use crate::segmenter::Segmenter;

/// The segmentation anchor track within a [`Media`]: the first video track, or —
/// for audio-only media — the first track. This mirrors [`Segmenter`]'s anchor
/// selection so that trim back-off and resegmentation agree on which track's
/// keyframes define segment (and trim) boundaries.
fn anchor_index(media: &Media) -> Option<usize> {
    if media.tracks.is_empty() {
        return None;
    }
    Some(
        media
            .tracks
            .iter()
            .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .unwrap_or(0),
    )
}

/// The composition (presentation) time of each sample of a track, in that
/// track's media timescale, from a zero decode-time base. Element `i` is
/// `sum(duration[0..i]) + composition_offset[i]`.
///
/// Returned as `i64` because `composition_offset` is signed and may push an
/// early sample's presentation time slightly negative.
fn presentation_times(track: &Track) -> Vec<i64> {
    let mut out = Vec::with_capacity(track.samples.len());
    let mut dts: i64 = 0;
    for s in &track.samples {
        out.push(dts + s.composition_offset as i64);
        dts += s.duration as i64;
    }
    out
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

/// Greatest common divisor (Euclid) — used to build a common tick base across
/// tracks with different media timescales when interleaving by decode time.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Least common multiple of two timescales (0 treated as 1).
fn lcm(a: u64, b: u64) -> u64 {
    let a = a.max(1);
    let b = b.max(1);
    a / gcd(a, b) * b
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
    /// `composition_offset`, from a zero base — lies in `[start, end)` is kept.
    ///
    /// To satisfy the CMAF constraint that a track opens on a random-access
    /// point (ISO/IEC 23000-19 §7.3.2.3), the **anchor** track (first video
    /// track, else the first track) is back-trimmed: the first sample it keeps
    /// is the nearest sync sample at or before the first sample the raw window
    /// would select. All other tracks keep exactly the samples inside the
    /// window (audio frames are all sync samples, so their first kept sample is
    /// already a random-access point).
    ///
    /// Output decode times are implicitly re-based to zero — the returned IR
    /// carries only the retained samples in order, and the muxers emit
    /// `base_media_decode_time = 0` for the first output segment.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `start >= end`, the media has no tracks, or the
    /// window selects no sample on any track.
    pub fn trim(&self, start: u64, end: u64) -> Result<Media> {
        if start >= end {
            return Err(Error::InvalidInput("trim: start must be < end"));
        }
        if self.tracks.is_empty() {
            return Err(Error::InvalidInput("trim: media has no tracks"));
        }
        let anchor = anchor_index(self).expect("non-empty media has an anchor");

        let mut out_tracks = Vec::with_capacity(self.tracks.len());
        let mut kept_any = false;
        for (ti, track) in self.tracks.iter().enumerate() {
            let ts = track.spec.timescale;
            let lo = rescale_floor(start, self.movie_timescale, ts);
            let hi = rescale_floor(end, self.movie_timescale, ts);
            let pts = presentation_times(track);

            // First sample whose presentation time is in [lo, hi): the raw
            // window lower edge. `last_in` is the exclusive upper index.
            let first_in = pts.iter().position(|&p| p >= lo && p < hi);
            let mut kept = Vec::new();
            if let Some(mut start_idx) = first_in {
                // Anchor: snap the start back to the preceding sync sample so the
                // output opens on a random-access point.
                if ti == anchor {
                    while start_idx > 0 && !track.samples[start_idx].is_sync {
                        start_idx -= 1;
                    }
                }
                for s in &track.samples[start_idx..] {
                    // Stop once presentation time reaches the window's upper edge.
                    // (Samples are appended in decode order; use their own pts.)
                    let idx = start_idx + kept.len();
                    if pts[idx] >= hi {
                        break;
                    }
                    kept.push(s.clone());
                }
            }
            if !kept.is_empty() {
                kept_any = true;
            }
            out_tracks.push(Track::new(track.spec.clone(), kept));
        }
        if !kept_any {
            return Err(Error::InvalidInput(
                "trim: window selected no samples on any track",
            ));
        }
        Ok(Media::new(out_tracks, self.movie_timescale))
    }

    /// Total duration of the segmentation anchor track (first video track, else
    /// the first track) in that track's media timescale — the denominator for
    /// how many segments a resegmentation at a given target will produce.
    ///
    /// Returns `(anchor_duration_ticks, anchor_timescale)`, or `None` for empty
    /// media.
    pub fn anchor_duration(&self) -> Option<(u64, u32)> {
        let anchor = anchor_index(self)?;
        let t = &self.tracks[anchor];
        let ticks: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
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
        // Per-track running decode time and a scale to a common tick base so
        // tracks with different timescales interleave correctly.
        let common = media
            .tracks
            .iter()
            .map(|t| t.spec.timescale as u64)
            .fold(1u64, lcm);
        let mut dts = alloc::vec![0u128; media.tracks.len()];
        loop {
            // Pick the track with a remaining sample of the smallest normalised
            // decode time (ties: lowest track index for determinism).
            let mut best: Option<usize> = None;
            let mut best_key = u128::MAX;
            for (ti, track) in media.tracks.iter().enumerate() {
                if cursors[ti] >= track.samples.len() {
                    continue;
                }
                let scale = (common / track.spec.timescale.max(1) as u64) as u128;
                let key = dts[ti] * scale;
                if key < best_key {
                    best_key = key;
                    best = Some(ti);
                }
            }
            let Some(ti) = best else { break };
            let track = &media.tracks[ti];
            let sample = &track.samples[cursors[ti]];
            seg.push(track.spec.track_id, sample.clone())?;
            dts[ti] += sample.duration as u128;
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
