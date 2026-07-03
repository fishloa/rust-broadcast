//! Low-Latency HLS — partial segments (CMAF chunks) + playlist directives.
//!
//! Reference: **RFC 8216bis** (HTTP Live Streaming 2nd Edition, draft-pantos-hls-rfc8216bis).
//!
//! Whole-segment CMAF ([`crate::segmenter::Segmenter`]) produces one
//! `styp`+`moof`+`mdat` per segment, so a client cannot begin a segment until the
//! whole thing exists — latency is at least one segment duration. **Low-Latency
//! HLS** cuts that by publishing each segment's **partial segments** ("parts",
//! RFC 8216bis §4.4.4.9) as they are produced: an independently addressable CMAF
//! chunk (a `moof`+`mdat` fragment) covering a sub-duration of the parent segment,
//! delivered before the parent segment finalizes.
//!
//! # Part structure (ISO/IEC 14496-12:2015)
//!
//! A part is the same fragment structure the batch builder
//! [`crate::build_media_segment`] emits, scoped to the samples of one sub-duration:
//! a bare `moof` (§8.8.4) + `mdat` (§8.1.1). Each part's `moof` carries its own
//! `mfhd.sequence_number` (§8.8.5) — contiguous and increasing across all parts —
//! and each track fragment's `tfdt.baseMediaDecodeTime` (§8.8.12) is the decode
//! time of that part's first sample. **Concatenating a segment's parts reproduces
//! exactly the coded samples, decode order, and per-track decode timeline of the
//! whole-segment [`crate::build_media_segment`] output** — parts split, they never
//! lose, duplicate, or reorder a sample. A part is marked
//! [`PartInfo::independent`] when its first sample is a sync sample
//! (`INDEPENDENT=YES` in the playlist, RFC 8216bis §4.4.4.9).
//!
//! Segment boundaries stay keyframe-aligned (each segment's first part begins on
//! the anchor track's sync sample), exactly as [`crate::segmenter::Segmenter`].
//!
//! # Playlist directives
//!
//! The playlist tags an LL-HLS client needs (`#EXT-X-SERVER-CONTROL`,
//! `#EXT-X-PART-INF`, `#EXT-X-PART`, `#EXT-X-PRELOAD-HINT`) are rendered by
//! [`crate::hls::MediaPlaylist`] when its
//! [`low_latency`](crate::hls::MediaPlaylist::low_latency) config is set; see
//! [`crate::hls`] for the exact RFC 8216bis syntax and sections.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::ll_dash::build_chunk;
use crate::pipeline::{build_init_segment, CodecConfig, FragmentTrackData, Sample, TrackSpec};

/// One finished LL-HLS **partial segment** ("part") — RFC 8216bis §4.4.4.9.
///
/// The `bytes` are an independent `moof`+`mdat` CMAF chunk; a caller writes them
/// to the part's own URI (`#EXT-X-PART:...,URI="<uri>"`) and can serve them
/// before the parent segment is complete.
#[derive(Debug, Clone)]
pub struct PartInfo {
    /// The part bytes: a bare `moof`+`mdat` fragment (no `styp`).
    pub bytes: Vec<u8>,
    /// Part duration in seconds (the anchor track's buffered duration for this
    /// part) — the `#EXT-X-PART:DURATION` value.
    pub duration: f64,
    /// `true` when this part's first anchor-track sample is a sync sample, so it
    /// begins with an independently decodable frame (`INDEPENDENT=YES`).
    pub independent: bool,
    /// 1-based sequence number of the parent segment this part belongs to.
    pub segment_seq: u32,
    /// 0-based index of this part within its parent segment.
    pub part_index: u32,
}

/// A finished full segment emitted by [`LlHlsSegmenter`] once its parts have all
/// been produced (the parent `styp`+`moof`+`mdat` covering the same samples).
#[derive(Debug, Clone)]
pub struct SegmentInfo {
    /// The whole-segment bytes (`styp`+`moof`+`mdat`), byte-identical to
    /// [`crate::build_media_segment`] for the segment's samples.
    pub bytes: Vec<u8>,
    /// Segment duration in seconds.
    pub duration: f64,
    /// 1-based sequence number of this segment.
    pub segment_seq: u32,
    /// Number of parts that made up this segment.
    pub part_count: u32,
}

/// Per-track accumulation state for the segment currently being built.
struct TrackState {
    spec: TrackSpec,
    /// Samples buffered for the current (not-yet-cut) segment, in decode order.
    /// These are the samples of the whole segment; they are only cleared at the
    /// segment boundary (parts drain a *view* via `part_start`, not the buffer).
    pending: Vec<Sample>,
    /// Index into `pending` of the first sample not yet emitted in a *part*.
    part_start: usize,
    /// `base_media_decode_time` (`tfdt`) of the next *part* for this track =
    /// decode time of `pending[part_start]`.
    part_base_decode: u64,
    /// `base_media_decode_time` of the whole current *segment* for this track =
    /// decode time of `pending[0]`.
    seg_base_decode: u64,
}

/// A stateful **Low-Latency HLS** segmenter (RFC 8216bis).
///
/// Same segmentation state machine as [`crate::segmenter::Segmenter`] — segments
/// are cut on the anchor track's sync samples once the segment target is reached —
/// but each segment is additionally sub-divided into **parts**: whenever the
/// anchor track's buffered-since-last-part duration reaches the *part target*, a
/// part (`moof`+`mdat`) is flushed and made available via
/// [`take_ready_parts`](Self::take_ready_parts) before the parent segment closes.
/// When the segment is cut, its remaining tail becomes a final part and the whole
/// segment is emitted via [`take_ready_segments`](Self::take_ready_segments).
///
/// ```
/// use transmux::{CodecConfig, Sample, TrackSpec};
/// use transmux::ll_hls::LlHlsSegmenter;
/// # fn spec() -> TrackSpec { unimplemented!() }
/// # fn au(sync: bool) -> Sample { Sample::from_raw(vec![0u8; 4], 3000) }
/// # if false {
/// // 1 s target segments, ~334 ms parts.
/// let mut seg = LlHlsSegmenter::with_part_target(vec![spec()], 1000, 1.0, 334).unwrap();
/// let init = seg.init_segment().unwrap();      // ftyp + moov
/// seg.push(1, au(true)).unwrap();              // keyframe
/// seg.push(1, au(false)).unwrap();
/// for part in seg.take_ready_parts() { /* write part.bytes over HTTP */ }
/// seg.flush().unwrap();                        // trailing part + segment
/// # }
/// ```
pub struct LlHlsSegmenter {
    tracks: Vec<TrackState>,
    movie_timescale: u32,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Target part duration in the *anchor track's* media timescale.
    part_target_ticks: u64,
    /// Buffered duration of the whole current segment on the anchor (ticks).
    anchor_seg_dur: u64,
    /// Buffered duration since the last part flush on the anchor (ticks).
    anchor_part_dur: u64,
    /// `mfhd.sequence_number` of the next part/segment `moof`, 1-based, contiguous.
    next_seq: u32,
    /// 1-based number of the segment currently being built.
    current_segment: u32,
    /// 0-based index of the next part within the current segment.
    next_part_index: u32,
    /// Parts finished but not yet taken by the caller.
    ready_parts: Vec<PartInfo>,
    /// Full segments finished but not yet taken by the caller.
    ready_segments: Vec<SegmentInfo>,
}

impl LlHlsSegmenter {
    /// Create an LL-HLS segmenter for `tracks`, cutting segments roughly every
    /// `target_duration_secs` on the anchor track's keyframes, and flushing a
    /// part whenever the anchor buffers `part_target_ms` milliseconds since the
    /// last part.
    ///
    /// The anchor is the first video track (falling back to the first track for
    /// audio-only), exactly as [`crate::segmenter::Segmenter`]. `movie_timescale`
    /// matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration_secs` is not positive and finite, or `part_target_ms` is 0.
    pub fn with_part_target(
        tracks: Vec<TrackSpec>,
        movie_timescale: u32,
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "ll-hls segmenter needs at least one track",
            ));
        }
        if !(target_duration_secs.is_finite() && target_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "target_duration_secs must be positive and finite",
            ));
        }
        if part_target_ms == 0 {
            return Err(Error::InvalidInput("part_target_ms must be >= 1"));
        }
        for (i, a) in tracks.iter().enumerate() {
            if tracks[i + 1..].iter().any(|b| b.track_id == a.track_id) {
                return Err(Error::InvalidInput("duplicate track_id"));
            }
        }

        let anchor = tracks
            .iter()
            .position(|t| matches!(t.config, CodecConfig::Avc { .. }))
            .unwrap_or(0);

        let anchor_timescale = tracks[anchor].timescale as f64;
        let target_ticks = ((target_duration_secs * anchor_timescale) as u64).max(1);
        // part_target_ms / 1000 * timescale, integer-safe.
        let part_target_ticks =
            ((part_target_ms as u64 * tracks[anchor].timescale as u64) / 1000).max(1);

        let tracks = tracks
            .into_iter()
            .map(|spec| TrackState {
                spec,
                pending: Vec::new(),
                part_start: 0,
                part_base_decode: 0,
                seg_base_decode: 0,
            })
            .collect();

        Ok(Self {
            tracks,
            movie_timescale,
            anchor,
            target_ticks,
            part_target_ticks,
            anchor_seg_dur: 0,
            anchor_part_dur: 0,
            next_seq: 1,
            current_segment: 1,
            next_part_index: 0,
            ready_parts: Vec::new(),
            ready_segments: Vec::new(),
        })
    }

    /// The part-target duration in seconds — the `#EXT-X-PART-INF:PART-TARGET`
    /// value a caller should advertise (see [`crate::hls::LowLatencyConfig`]).
    pub fn part_target_secs(&self) -> f64 {
        self.part_target_ticks as f64 / self.tracks[self.anchor].spec.timescale as f64
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`). Stable for
    /// the life of the segmenter; write it once before any part or segment.
    pub fn init_segment(&self) -> Result<Vec<u8>> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        build_init_segment(&specs, self.movie_timescale)
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// When the anchor track reaches a sync sample past the segment target, the
    /// current segment is finalized (its trailing samples flushed as a final part,
    /// then the whole segment emitted) *before* this sample is buffered, so the
    /// new keyframe opens the next segment's first part on a random-access point.
    /// Otherwise, once the anchor buffers a part-target's worth of samples since
    /// the last part, a part is flushed.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, or a part/segment
    /// build fails.
    pub fn push(&mut self, track_id: u32, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        // Segment boundary: anchor keyframe past target → finalize the segment.
        if idx == self.anchor
            && sample.is_sync
            && self.anchor_seg_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty()
        {
            self.finish_segment()?;
        }

        if idx == self.anchor {
            self.anchor_seg_dur += sample.duration as u64;
            self.anchor_part_dur += sample.duration as u64;
        }
        self.tracks[idx].pending.push(sample);

        // Part boundary: the anchor buffered a full part's worth since the last
        // part, but the segment is not yet due to close. Hold the segment's final
        // part for the boundary/flush so trailing non-anchor samples ride it.
        if idx == self.anchor
            && self.anchor_part_dur >= self.part_target_ticks
            && self.anchor_seg_dur < self.target_ticks
        {
            self.emit_part(false)?;
        }
        Ok(())
    }

    /// Finalize the trailing segment at end-of-stream. A no-op if nothing is
    /// buffered. Emits the final part(s) and the whole segment.
    ///
    /// # Errors
    /// Propagates a part/segment build failure.
    pub fn flush(&mut self) -> Result<()> {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.finish_segment()?;
        }
        Ok(())
    }

    /// Remove and return every part finished since the last call, in order.
    pub fn take_ready_parts(&mut self) -> Vec<PartInfo> {
        core::mem::take(&mut self.ready_parts)
    }

    /// Remove and return every full segment finished since the last call, in
    /// order — distinct from the parts.
    pub fn take_ready_segments(&mut self) -> Vec<SegmentInfo> {
        core::mem::take(&mut self.ready_segments)
    }

    /// Finalize the current segment: flush its remaining un-parted samples as a
    /// final part, emit the whole segment, then open the next segment.
    fn finish_segment(&mut self) -> Result<()> {
        // Any samples buffered since the last part become the segment's final
        // part (also carries trailing non-anchor samples).
        let has_tail = self.tracks.iter().any(|t| t.part_start < t.pending.len());
        if has_tail {
            self.emit_part(true)?;
        }

        // Emit the whole segment from all buffered samples (byte-identical to the
        // batch build for the same sample set).
        let seg_seq = self.next_seq;
        let seg_bytes = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .filter(|t| !t.pending.is_empty())
                .map(|t| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.seg_base_decode,
                    samples: &t.pending,
                })
                .collect();
            crate::pipeline::build_media_segment(seg_seq, &frags)?
        };
        self.next_seq += 1;

        let seg_duration =
            self.anchor_seg_dur as f64 / self.tracks[self.anchor].spec.timescale as f64;
        self.ready_segments.push(SegmentInfo {
            bytes: seg_bytes,
            duration: seg_duration,
            segment_seq: self.current_segment,
            part_count: self.next_part_index,
        });

        // Advance per-track decode times past the whole segment and clear buffers.
        for t in &mut self.tracks {
            let dur: u64 = t.pending.iter().map(|s| s.duration as u64).sum();
            t.seg_base_decode += dur;
            t.part_base_decode = t.seg_base_decode;
            t.pending.clear();
            t.part_start = 0;
        }
        self.anchor_seg_dur = 0;
        self.anchor_part_dur = 0;
        self.current_segment += 1;
        self.next_part_index = 0;
        Ok(())
    }

    /// Emit one part covering the samples buffered since the last part. If
    /// `final_part`, drain every remaining sample of every track; otherwise drain
    /// only the anchor's samples since the last part (non-anchor samples ride the
    /// segment's final part, matching the whole-segment single per-track run).
    fn emit_part(&mut self, final_part: bool) -> Result<()> {
        let anchor = self.anchor;

        // Per track, the [part_start .. end) sample range this part drains.
        let take_ends: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if final_part || i == anchor {
                    t.pending.len()
                } else {
                    t.part_start
                }
            })
            .collect();

        // Nothing new for any track → skip (keeps sequence numbers meaningful).
        if take_ends
            .iter()
            .zip(&self.tracks)
            .all(|(&end, t)| end == t.part_start)
        {
            return Ok(());
        }

        // Independence: the part's first anchor sample is a sync sample.
        let anchor_state = &self.tracks[anchor];
        let independent = anchor_state
            .pending
            .get(anchor_state.part_start)
            .map(|s| s.is_sync)
            .unwrap_or(false);

        // Part duration = the anchor's buffered-since-last-part duration.
        let part_secs = self.anchor_part_dur as f64 / self.tracks[anchor].spec.timescale as f64;

        let seq = self.next_seq;
        let part_bytes = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .zip(&take_ends)
                .filter(|(t, &end)| end > t.part_start)
                .map(|(t, &end)| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.part_base_decode,
                    samples: &t.pending[t.part_start..end],
                })
                .collect();
            // A part is a bare moof+mdat (no styp): with_styp = false.
            build_chunk(seq, &frags, false)?
        };
        self.next_seq += 1;

        // Advance per-track part cursors and part-base decode times.
        for (t, &end) in self.tracks.iter_mut().zip(&take_ends) {
            let dur: u64 = t.pending[t.part_start..end]
                .iter()
                .map(|s| s.duration as u64)
                .sum();
            t.part_base_decode += dur;
            t.part_start = end;
        }

        let part_index = self.next_part_index;
        self.next_part_index += 1;
        self.anchor_part_dur = 0;

        self.ready_parts.push(PartInfo {
            bytes: part_bytes,
            duration: part_secs,
            independent,
            segment_seq: self.current_segment,
            part_index,
        });
        Ok(())
    }
}
