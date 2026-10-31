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
//! [`broadcast_hls::MediaPlaylist`] when its
//! [`low_latency`](broadcast_hls::MediaPlaylist::low_latency) config is set;
//! see the `broadcast-hls` crate for the exact RFC 8216bis syntax and sections.

use crate::TrackId;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::{Demand, Stage, Timestamp};

use crate::error::{Error, Result};
use crate::ll_dash::build_chunk;
use crate::pipeline::{FragmentTrackData, Sample, TrackSpec, build_init_segment};
use crate::segmenter::{
    MAX_PENDING_SAMPLES_PER_TRACK, MediaClock, choose_anchor, no_sync_sample_error,
};

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

/// A typed external-cut plan fed to a follower [`LlHlsSegmenter`] so it cuts
/// its segments and parts at the leader's boundaries (issue #1428 slice 2a).
///
/// One plan describes one leader segment: its media sequence number, its start
/// decode time in the leader's timescale, and (for LL-HLS) the part boundaries
/// within it. The follower converts every time with ceiling rounding into its
/// own tracks' timescales (see [`LlHlsSegmenter::follow`]).
#[derive(Debug, Clone)]
pub struct CutPlan {
    /// Media sequence number of this segment (must match the follower's).
    pub segment_seq: u32,
    /// Decode time of the segment's first sample in the leader's timescale.
    pub segment_start: i64,
    /// Timescale of [`Self::segment_start`] and of any [`PartCut`] whose own
    /// timescale is zero.
    pub timescale: u32,
    /// Part boundaries within this segment. Empty means "one part for the whole
    /// segment".
    pub parts: Vec<PartCut>,
}

/// One part boundary within a [`CutPlan`].
#[derive(Debug, Clone)]
pub struct PartCut {
    /// Part index within the parent segment.
    pub part_index: u32,
    /// Decode time of the part's first sample in the leader's timescale.
    pub start: i64,
    /// Timescale of [`Self::start`] (zero inherits the plan's timescale).
    pub timescale: u32,
}

impl CutPlan {
    /// A plan for the leader segment `segment_seq` starting at `segment_start`.
    pub fn new(segment_seq: u32, segment_start: i64, timescale: u32, parts: Vec<PartCut>) -> Self {
        Self {
            segment_seq,
            segment_start,
            timescale,
            parts,
        }
    }
}

impl PartCut {
    /// A part boundary at `start` (in `timescale`; zero inherits the plan's).
    pub fn new(part_index: u32, start: i64, timescale: u32) -> Self {
        Self {
            part_index,
            start,
            timescale,
        }
    }

    fn effective_timescale(&self, plan_timescale: u32) -> u32 {
        if self.timescale == 0 {
            plan_timescale
        } else {
            self.timescale
        }
    }
}

/// A part boundary handed out by a leader [`LlHlsSegmenter`] as soon as that
/// part is cut, so a follower can cut the matching part without waiting for
/// the whole segment to close (issue #1428 part-level low-latency audio).
#[derive(Debug, Clone, Copy)]
pub struct PartCutReady {
    /// Media sequence number of the segment this part belongs to.
    pub msn: u32,
    /// Part index within that segment.
    pub part_index: u32,
    /// Decode time of the part's first sample, in the leader's timescale.
    pub start_dts: i64,
    /// Timescale of [`Self::start_dts`].
    pub timescale: u32,
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
    /// Advances `part_base_decode` past each sample as it is drained into a
    /// part, under the same duration-then-dts-delta rule as the anchor
    /// accumulator (see [`MediaClock`]).
    ///
    /// Deliberately a *different* clock instance from `seg_clock`: the two
    /// accumulators cover the same samples at different granularities (parts
    /// walk `pending` in sub-ranges, the segment walks it once at the
    /// boundary), and a [`MediaClock`]'s dts-delta fallback is only correct
    /// if it sees each sample exactly once. A plain `duration` sum would pin
    /// both at 0 forever on a `duration: Some(0)` stream, collapsing every
    /// part's and segment's `tfdt` onto the same decode time.
    part_clock: MediaClock,
    /// Advances `seg_base_decode` past the whole segment at the boundary —
    /// see `part_clock` for why this is a second, independent clock.
    seg_clock: MediaClock,
    /// Whether the shared-origin offset has been applied to this track's
    /// first emitted follower segment.
    origin_applied: bool,
    /// A follower segment closed with no sample of this track, so the next
    /// one must restart its `tfdt` from its own first sample's decode time
    /// rather than continue contiguously.
    gap_pending: bool,
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
/// # use transmux::{AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps};
/// # fn spec() -> TrackSpec {
/// #     let record = AVCDecoderConfigurationRecord {
/// #         configuration_version: 1,
/// #         profile_indication: 66,
/// #         profile_compatibility: 0,
/// #         level_indication: 30,
/// #         length_size_minus_one: 3,
/// #         sps: vec![AvcSps(vec![0x67, 0x42, 0xc0, 0x1e, 0xd9, 0x00, 0x80, 0x1e, 0x24])],
/// #         pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
/// #         chroma_format: None,
/// #         bit_depth_luma_minus8: None,
/// #         bit_depth_chroma_minus8: None,
/// #         sps_ext: vec![],
/// #     };
/// #     TrackSpec::new(transmux::TrackId::new(1), 90_000, CodecConfig::Avc {
/// #         config: AVCConfigurationBox::new(record),
/// #         width: 16,
/// #         height: 16,
/// #     })
/// # }
/// # fn au(sync: bool) -> Sample {
/// #     use std::sync::atomic::{AtomicI64, Ordering};
/// #     static NEXT_DTS: AtomicI64 = AtomicI64::new(0);
/// #     let dts = NEXT_DTS.fetch_add(1000, Ordering::Relaxed);
/// #     Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(1000), sync)
/// # }
/// // 1 s target segments, ~334 ms parts.
/// let mut seg = LlHlsSegmenter::with_part_target(vec![spec()], 1000, 1.0, 334).unwrap();
/// let init = seg.init_segment().unwrap();      // ftyp + moov
/// assert_eq!(&init[4..8], b"ftyp");
/// seg.push(transmux::TrackId::new(1), au(true)).unwrap();              // keyframe
/// seg.push(transmux::TrackId::new(1), au(false)).unwrap();
/// assert!(seg.take_ready_parts().is_empty());  // still buffering: below the part target
/// seg.flush().unwrap();                        // trailing part + segment
/// assert_eq!(seg.take_ready_parts().len(), 1);
/// assert_eq!(seg.take_ready_segments().len(), 1);
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
    /// Anchor-progress clock: advances *both* accumulators above from each
    /// anchor sample's `duration`, or from its `dts` delta when `duration` is
    /// absent or zero (see [`MediaClock`]).
    anchor_clock: MediaClock,
    /// `mfhd.sequence_number` of the next part/segment `moof`, 1-based, contiguous.
    next_seq: u32,
    /// 1-based number of the segment currently being built.
    current_segment: u32,
    /// 0-based index of the next part within the current segment.
    next_part_index: u32,
    /// Parts and full segments finished but not yet taken by the caller, in
    /// the exact order they were cut. The single source of truth for the
    /// inherent [`take_ready_parts`](Self::take_ready_parts)/
    /// [`take_ready_segments`](Self::take_ready_segments) drains *and*
    /// [`Stage::poll`] — there is no separate staging copy. The inherent
    /// drains filter this queue by variant (each leaving the other kind's
    /// relative order undisturbed); `poll` pops the raw item. Whichever API
    /// (or mix of both) the caller uses, every part and segment is delivered
    /// exactly once.
    ready: VecDeque<LlHlsStageOutput>,
    /// Follower mode: cut only at externally supplied boundaries.
    external_cuts: bool,
    timeline_origin: Option<(i64, u32)>,
    /// Leader mode: record the boundaries of every cut into `ready_cuts`.
    record_cuts: bool,
    /// Part boundaries of the segment being built (leader mode).
    current_cuts: Vec<PartCut>,
    /// Finished segments' boundaries not yet taken (leader mode).
    ready_cuts: VecDeque<CutPlan>,
    /// Leader mode: hand out each part boundary as the part closes.
    record_part_cuts: bool,
    /// Part boundaries handed out as parts close (leader mode), not yet taken.
    ready_part_cuts: VecDeque<PartCutReady>,
    /// Plan for the segment currently being built (its start time and parts).
    current_plan: Option<CutPlan>,
    /// Follower mode: the open segment is fed part by part via `follow_part`.
    incremental: bool,
    /// Index into the open incremental segment's parts of the next one to emit.
    inc_next_part: usize,
    /// Parts actually emitted for the open incremental segment.
    inc_part_count: u32,
}

impl LlHlsSegmenter {
    /// Create an LL-HLS segmenter for `tracks`, cutting segments roughly every
    /// `target_duration_secs` on the anchor track's keyframes, and flushing a
    /// part whenever the anchor buffers `part_target_ms` milliseconds since the
    /// last part.
    ///
    /// The anchor is chosen by the shared
    /// [`crate::segmenter::choose_anchor`] — the first **video**
    /// track (any codec, not just AVC), falling back to the first
    /// anchor-capable track — exactly as [`crate::segmenter::Segmenter`].
    /// `movie_timescale` matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration_secs` is not positive and finite, `part_target_ms` is 0,
    /// or no track is anchor-capable (every track is section-carried).
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

        // MUX = strict but filterable (media plane step-2 fix wave 1,
        // B2-B4): a track `build_init_segment` cannot place into an ISOBMFF
        // `trak` (opaque `CodecConfig::Data` or `CodecConfig::Subtitle`) is
        // no longer silently omitted here — it surfaces the same named
        // error every other mux entry point does, the first time
        // `init_segment`/a part build actually needs the `trak`. The caller
        // must pre-filter first (e.g. with
        // `tracks.retain(|t| t.config.is_muxable_in_bmff())`) if it wants
        // to drop such tracks rather than fail.

        // Anchor = first **video** track (any `CodecConfig::is_video` codec),
        // else the first anchor-capable track — the shared rule
        // `Segmenter`/`ts_hls` use. This used to match only
        // `CodecConfig::Avc`, so an HEVC+AAC media with audio first anchored on
        // the *audio* track: segments did not begin on an IRAP and every part
        // was reported `independent` (audio is all sync samples), advertising
        // `INDEPENDENT=YES` on parts that actually start mid-GOP.
        let anchor = choose_anchor(tracks.iter().map(|t| &t.config))?;

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
                part_clock: MediaClock::new(),
                seg_clock: MediaClock::new(),
                origin_applied: false,
                gap_pending: false,
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
            anchor_clock: MediaClock::new(),
            next_seq: 1,
            current_segment: 1,
            next_part_index: 0,
            ready: VecDeque::new(),
            external_cuts: false,
            timeline_origin: None,
            record_cuts: false,
            current_cuts: Vec::new(),
            ready_cuts: VecDeque::new(),
            record_part_cuts: false,
            ready_part_cuts: VecDeque::new(),
            current_plan: None,
            incremental: false,
            inc_next_part: 0,
            inc_part_count: 0,
        })
    }

    /// The part-target duration in seconds — the `#EXT-X-PART-INF:PART-TARGET`
    /// value a caller should advertise (see [`broadcast_hls::LowLatencyConfig`]).
    pub fn part_target_secs(&self) -> f64 {
        // `timescale` is a wire `u32` (`mdhd.timescale`, ISO/IEC
        // 14496-12:2015 §8.4.2) — `.max(1)` guards the same zero-timescale
        // case `ts_hls.rs` guards at every analogous division (e.g.
        // `ts_hls.rs:225`), so a malformed zero timescale can't turn this
        // into `inf`/`NaN` rendered straight into `#EXT-X-PART-INF` (audit
        // finding #5).
        self.part_target_ticks as f64 / self.tracks[self.anchor].spec.timescale.max(1) as f64
    }

    /// The `(next_seq, current_segment)` pair this segmenter would give the
    /// next emitted part and segment — the values `next_seq` (the next
    /// `mfhd.sequence_number`) and `current_segment` (the 1-based number of
    /// the segment currently being built, which becomes the next
    /// [`SegmentInfo::segment_seq`]). A caller that rebuilds the segmenter
    /// mid-stream (e.g. after a track-set change, issue #781) seeds the new
    /// one from these so sequence numbering is **strictly monotonic** across
    /// the rebuild — no `EXT-X-MEDIA-SEQUENCE` reset, no client playlists
    /// rewinding.
    pub fn next_sequence_numbers(&self) -> (u32, u32) {
        (self.next_seq, self.current_segment)
    }

    /// Build a segmenter whose sequence numbering starts from `next_seq`
    /// (`mfhd.sequence_number`) and `current_segment` (1-based segment
    /// number), resuming where a previous segmenter left off — the seeding
    /// side of [`Self::next_sequence_numbers`] (issue #781).
    ///
    /// The usual constructor, [`Self::with_part_target`], starts both at 1.
    #[doc(hidden)]
    pub fn with_part_target_at(
        tracks: Vec<TrackSpec>,
        movie_timescale: u32,
        target_duration_secs: f64,
        part_target_ms: u32,
        next_seq: u32,
        current_segment: u32,
    ) -> Result<Self> {
        let mut seg = Self::with_part_target(
            tracks,
            movie_timescale,
            target_duration_secs,
            part_target_ms,
        )?;
        seg.next_seq = next_seq;
        seg.current_segment = current_segment;
        // Restart the part counter at 0 (the new segment being built is
        // fresh), but the segment *number* resumes from where the old
        // segmenter's `current_segment` was (it was already incremented
        // past the last emitted segment).
        seg.next_part_index = 0;
        Ok(seg)
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
    /// Both accumulators advance via [`MediaClock`]: each anchor sample's own
    /// `duration` when that is a real, non-zero span, else the `dts` delta from
    /// the previous anchor sample — `duration` alone stalls on the `Some(0)`
    /// durations live input legitimately produces (no part *and* no segment
    /// would ever be emitted).
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, a part/segment
    /// build fails, or this track already holds
    /// [`MAX_PENDING_SAMPLES_PER_TRACK`] un-cut samples (no anchor sync sample
    /// to cut on — call [`flush`](Self::flush) to close a trailing partial
    /// segment).
    pub fn push(&mut self, track_id: TrackId, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        if self.external_cuts {
            if sample.dts.is_none() {
                return Err(Error::InvalidInput(
                    "follower segmenter push: sample has no dts",
                ));
            }
            if self.tracks[idx].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
                return Err(Error::InvalidInput(
                    "follower segmenter pending buffer full: no boundary received within \
                     MAX_PENDING_SAMPLES_PER_TRACK samples",
                ));
            }
            self.tracks[idx].pending.push(sample);
            return self.try_emit_inc_parts();
        }

        // Segment boundary: anchor keyframe past target → finalize the segment.
        if idx == self.anchor
            && sample.flags.is_sync
            && self.anchor_seg_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty()
        {
            self.finish_segment()?;
        }

        if self.tracks[idx].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
            return Err(no_sync_sample_error());
        }

        if idx == self.anchor {
            let elapsed = self.anchor_clock.tick(&sample);
            self.anchor_seg_dur += elapsed;
            self.anchor_part_dur += elapsed;
        }
        self.tracks[idx].pending.push(sample);

        // Part boundary: the anchor's buffered-since-last-part span has reached
        // the part target. `emit_part` then drains only the samples that fit
        // *within* the part target (RFC 8216bis §4.4.4.9: "The duration of a
        // Partial Segment MUST be less than or equal to the Part Target
        // Duration"), leaving the crossing sample for the next part. This holds
        // past the segment target too: a segment waiting for its next keyframe
        // keeps cutting parts, so its final part is only the short remainder.
        if idx == self.anchor && self.anchor_part_dur >= self.part_target_ticks {
            self.emit_part(false)?;
        }
        Ok(())
    }

    /// Start (or continue) **follower mode**: this segmenter stops cutting on its
    /// own and cuts exactly where a leader segmenter cut, so its segments and
    /// parts carry the leader's sequence numbers and boundaries (RFC 8216bis
    /// §4.4.4.9 matching Media Sequence Numbers and Parts across Renditions).
    ///
    /// Call once per leader segment, in order, as the leader finishes it. The
    /// call registers `plan` as the segment now being built and closes the
    /// previously registered one: its segment (and parts) are emitted from the
    /// buffered samples whose decode time is strictly less than `plan`'s start.
    /// The last registered segment is closed by [`flush`](Self::flush).
    ///
    /// Boundary rule: a sample belongs to the segment (and part) in which it
    /// *starts*; a sample whose decode time equals a boundary opens the next
    /// one. Boundaries are converted into each track's timescale with exact
    /// integer arithmetic, rounded up, which is exact for integer decode times
    /// (`d < ceil(x)` iff `d < x`). Samples earlier than the first registered
    /// start join the first segment's first part, so nothing is dropped.
    ///
    /// A segment (or part) in which no follower sample starts is not emitted,
    /// so its sequence number (or part index) is absent from the output; the
    /// samples around it are unaffected.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if a timescale is zero, `plan` does not advance
    /// the previous plan's sequence number and start, its part starts are not
    /// non-decreasing, or this segmenter already cut on its own before the
    /// first `follow` call.
    pub fn follow(&mut self, plan: CutPlan) -> Result<()> {
        if self.incremental {
            self.close_incremental_segment(Some((plan.segment_start, plan.timescale)))?;
        }
        if plan.timescale == 0 {
            return Err(Error::InvalidInput("follow: plan timescale must be > 0"));
        }
        if self.tracks.iter().any(|t| t.spec.timescale == 0) {
            return Err(Error::InvalidInput("follow: track timescale must be > 0"));
        }
        let mut previous_part: Option<(i64, u32)> = None;
        for p in &plan.parts {
            let ts = p.effective_timescale(plan.timescale);
            if let Some((start, prev_ts)) = previous_part
                && cmp_time(p.start, ts, start, prev_ts).is_lt()
            {
                return Err(Error::InvalidInput(
                    "follow: part starts must be non-decreasing",
                ));
            }
            previous_part = Some((p.start, ts));
        }
        let cut_on_own = !self.ready.is_empty()
            || self
                .tracks
                .iter()
                .any(|t| t.part_start > 0 || t.seg_base_decode > 0);
        if !self.external_cuts && cut_on_own {
            return Err(Error::InvalidInput(
                "follow: segmenter already cut on its own; call follow before the first cut",
            ));
        }
        if let Some(current) = &self.current_plan
            && (plan.segment_seq <= current.segment_seq
                || cmp_time(
                    plan.segment_start,
                    plan.timescale,
                    current.segment_start,
                    current.timescale,
                )
                .is_lt())
        {
            return Err(Error::InvalidInput(
                "follow: plan must advance the previous plan's sequence number and start",
            ));
        }
        self.external_cuts = true;
        if let Some(current) = self.current_plan.take() {
            let end = (plan.segment_start, plan.timescale);
            self.cut_follower_segment(&current, Some(end))?;
        }
        self.current_segment = plan.segment_seq;
        self.current_plan = Some(plan);
        Ok(())
    }

    /// Feed one part boundary to a follower as soon as the leader cuts that
    /// part (issue #1428 part-level low-latency audio), the incremental
    /// counterpart of [`Self::follow`].
    ///
    /// The first call for a `segment_seq` opens that segment; a later call with
    /// a greater `segment_seq` closes the open one at the new boundary. A part
    /// is emitted as soon as a buffered sample starts at or after the next
    /// part's boundary, never earlier, so a part is only cut once the follower
    /// holds every sample it will contain. Parts and segments are byte-identical
    /// to what `follow` produces for the same boundaries. The last open segment
    /// is closed by [`flush`](Self::flush).
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if a timescale is zero, `segment_seq` or the part
    /// starts go backwards, this segmenter already cut on its own, or it is
    /// following whole-segment plans through [`follow`](Self::follow).
    pub fn follow_part(
        &mut self,
        segment_seq: u32,
        part_index: u32,
        start: i64,
        timescale: u32,
    ) -> Result<()> {
        if timescale == 0 {
            return Err(Error::InvalidInput("follow_part: timescale must be > 0"));
        }
        if self.tracks.iter().any(|t| t.spec.timescale == 0) {
            return Err(Error::InvalidInput(
                "follow_part: track timescale must be > 0",
            ));
        }
        if !self.incremental {
            let cut_on_own = !self.ready.is_empty()
                || self
                    .tracks
                    .iter()
                    .any(|t| t.part_start > 0 || t.seg_base_decode > 0);
            if !self.external_cuts && cut_on_own {
                return Err(Error::InvalidInput(
                    "follow_part: segmenter already cut on its own; call follow_part before the first cut",
                ));
            }
            if self.current_plan.is_some() {
                return Err(Error::InvalidInput(
                    "follow_part: segmenter is following whole-segment plans",
                ));
            }
        }
        if self.incremental
            && let Some(open) = &self.current_plan
        {
            if segment_seq < open.segment_seq {
                return Err(Error::InvalidInput(
                    "follow_part: segment_seq must not go backwards",
                ));
            }
            if segment_seq > open.segment_seq {
                self.close_incremental_segment(Some((start, timescale)))?;
            }
        }
        if !self.incremental {
            self.external_cuts = true;
            self.incremental = true;
            self.inc_next_part = 0;
            self.inc_part_count = 0;
            self.current_segment = segment_seq;
            self.current_plan = Some(CutPlan::new(segment_seq, start, timescale, Vec::new()));
        }
        let plan = self
            .current_plan
            .as_mut()
            .ok_or(Error::InvalidInput("follow_part: no open segment"))?;
        if let Some(last) = plan.parts.last()
            && cmp_time(
                start,
                timescale,
                last.start,
                last.effective_timescale(plan.timescale),
            )
            .is_lt()
        {
            return Err(Error::InvalidInput(
                "follow_part: part starts must be non-decreasing",
            ));
        }
        plan.parts.push(PartCut::new(part_index, start, timescale));
        self.try_emit_inc_parts()
    }

    fn try_emit_inc_parts(&mut self) -> Result<()> {
        while let Some((segment_seq, part_index, next)) = self.next_inc_part() {
            let anchor = &self.tracks[self.anchor];
            let threshold = ceil_rescale(next.0, next.1, anchor.spec.timescale);
            let reached = anchor.pending[anchor.part_start..]
                .iter()
                .any(|s| follower_dts(s) >= threshold);
            if !reached {
                break;
            }
            self.emit_inc_part(segment_seq, part_index, Some(next))?;
            self.inc_next_part += 1;
        }
        Ok(())
    }

    /// The open segment's next unemitted part and the boundary that ends it,
    /// once that boundary is known.
    fn next_inc_part(&self) -> Option<(u32, u32, (i64, u32))> {
        if !self.incremental {
            return None;
        }
        let plan = self.current_plan.as_ref()?;
        let part = plan.parts.get(self.inc_next_part)?;
        let next = plan.parts.get(self.inc_next_part + 1)?;
        Some((
            plan.segment_seq,
            part.part_index,
            (next.start, next.effective_timescale(plan.timescale)),
        ))
    }

    /// Emit the samples from each track's `part_start` up to `end` (all of
    /// them when `None`) as one part. Returns whether a part was produced.
    fn emit_inc_part(
        &mut self,
        segment_seq: u32,
        part_index: u32,
        end: Option<(i64, u32)>,
    ) -> Result<bool> {
        let take_ends: Vec<usize> = self
            .tracks
            .iter()
            .map(|t| match end {
                Some((value, ts)) => {
                    let threshold = ceil_rescale(value, ts, t.spec.timescale);
                    t.part_start
                        + t.pending[t.part_start..].partition_point(|s| follower_dts(s) < threshold)
                }
                None => t.pending.len(),
            })
            .collect();
        let active: Vec<bool> = self
            .tracks
            .iter()
            .zip(&take_ends)
            .map(|(t, &e)| e > t.part_start)
            .collect();
        if !active.contains(&true) {
            return Ok(false);
        }
        self.apply_follower_origin(&active);

        let frags: Vec<FragmentTrackData<'_>> = self
            .tracks
            .iter()
            .zip(&take_ends)
            .filter(|&(t, &e)| e > t.part_start)
            .map(|(t, &e)| FragmentTrackData {
                track_id: t.spec.track_id,
                base_media_decode_time: t.part_base_decode,
                samples: &t.pending[t.part_start..e],
            })
            .collect();
        let bytes = build_chunk(self.next_seq, &frags, false)?;
        self.next_seq += 1;
        let independent = frags
            .first()
            .and_then(|f| f.samples.first())
            .is_some_and(|s| s.flags.is_sync);
        let first_ti = active
            .iter()
            .position(|&a| a)
            .ok_or(Error::InvalidInput("follow_part: part has no track"))?;
        let duration = span_secs(
            &self.tracks[first_ti],
            self.tracks[first_ti].part_start,
            take_ends[first_ti],
        );

        for (t, &e) in self.tracks.iter_mut().zip(&take_ends) {
            let mut clock = t.part_clock;
            let ticks: u64 = t.pending[t.part_start..e]
                .iter()
                .map(|s| clock.tick(s))
                .sum();
            t.part_clock = clock;
            t.part_base_decode += ticks;
            t.part_start = e;
        }
        self.inc_part_count += 1;
        self.ready.push_back(LlHlsStageOutput::Part(PartInfo {
            bytes,
            duration,
            independent,
            segment_seq,
            part_index,
        }));
        Ok(true)
    }

    /// Close the open incremental segment: emit its remaining parts (bounded
    /// by `end`, the next segment's start, when known) and then the whole
    /// segment over every sample starting before `end`.
    fn close_incremental_segment(&mut self, end: Option<(i64, u32)>) -> Result<()> {
        let Some(plan) = self.current_plan.take() else {
            self.incremental = false;
            return Ok(());
        };
        let starts: Vec<(u32, (i64, u32))> = plan
            .parts
            .iter()
            .map(|p| {
                (
                    p.part_index,
                    (p.start, p.effective_timescale(plan.timescale)),
                )
            })
            .collect();
        for k in self.inc_next_part..starts.len() {
            let part_end = starts.get(k + 1).map(|s| s.1).or(end);
            self.emit_inc_part(plan.segment_seq, starts[k].0, part_end)?;
        }
        let splits: Vec<usize> = self
            .tracks
            .iter()
            .map(|t| match end {
                Some((value, ts)) => {
                    let threshold = ceil_rescale(value, ts, t.spec.timescale);
                    t.pending.partition_point(|s| follower_dts(s) < threshold)
                }
                None => t.pending.len(),
            })
            .collect();
        let part_count = self.inc_part_count;
        self.incremental = false;
        self.inc_next_part = 0;
        self.inc_part_count = 0;
        self.finish_follower_segment(plan.segment_seq, &splits, part_count)
    }

    /// Finalize the trailing segment at end-of-stream. A no-op if nothing is
    /// buffered. Emits the final part(s) and the whole segment. In follower
    /// mode the registered plan's segment takes every remaining sample.
    ///
    /// # Errors
    /// Propagates a part/segment build failure; in follower mode also
    /// [`Error::InvalidInput`] if samples are buffered with no registered plan.
    pub fn flush(&mut self) -> Result<()> {
        if self.incremental {
            return self.close_incremental_segment(None);
        }
        if self.external_cuts {
            return match self.current_plan.take() {
                Some(current) => self.cut_follower_segment(&current, None),
                None if self.tracks.iter().any(|t| !t.pending.is_empty()) => Err(
                    Error::InvalidInput("flush: follower has buffered samples but no cut plan"),
                ),
                None => Ok(()),
            };
        }
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.finish_segment()?;
        }
        Ok(())
    }

    /// Emit the segment described by `plan`: every buffered sample starting
    /// before `end` (all of them when `end` is `None`), split into parts at
    /// the plan's part starts.
    fn cut_follower_segment(&mut self, plan: &CutPlan, end: Option<(i64, u32)>) -> Result<()> {
        let part_starts: Vec<(u32, i64, u32)> = if plan.parts.is_empty() {
            vec![(0, plan.segment_start, plan.timescale)]
        } else {
            plan.parts
                .iter()
                .map(|p| (p.part_index, p.start, p.effective_timescale(plan.timescale)))
                .collect()
        };
        let nparts = part_starts.len();

        // bounds[track][k]..bounds[track][k + 1] is part k's sample range; the
        // first part also takes samples before its own start.
        let mut bounds: Vec<Vec<usize>> = Vec::with_capacity(self.tracks.len());
        for t in &self.tracks {
            let before = |limit: Option<(i64, u32)>, upto: usize| match limit {
                Some((value, ts)) => {
                    let threshold = ceil_rescale(value, ts, t.spec.timescale);
                    t.pending[..upto].partition_point(|s| follower_dts(s) < threshold)
                }
                None => upto,
            };
            let split = before(end, t.pending.len());
            let mut b = Vec::with_capacity(nparts + 1);
            b.push(0);
            for &(_, start, ts) in &part_starts[1..] {
                b.push(before(Some((start, ts)), split));
            }
            b.push(split);
            bounds.push(b);
        }

        let active: Vec<bool> = bounds.iter().map(|b| b[nparts] > 0).collect();
        self.apply_follower_origin(&active);

        let mut cumulative: Vec<Vec<u64>> = Vec::with_capacity(self.tracks.len());
        for (t, b) in self.tracks.iter().zip(&bounds) {
            let mut clock = MediaClock::resumed_at(t.seg_clock.last_dts());
            let mut cum = Vec::with_capacity(b[nparts] + 1);
            cum.push(0u64);
            for s in &t.pending[..b[nparts]] {
                let last = cum[cum.len() - 1];
                cum.push(last + clock.tick(s));
            }
            cumulative.push(cum);
        }

        let seg_seq = plan.segment_seq;
        let mut emitted_parts: u32 = 0;
        for (k, &(part_index, _, _)) in part_starts.iter().enumerate() {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .enumerate()
                .filter(|(ti, _)| bounds[*ti][k + 1] > bounds[*ti][k])
                .map(|(ti, t)| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.seg_base_decode + cumulative[ti][bounds[ti][k]],
                    samples: &t.pending[bounds[ti][k]..bounds[ti][k + 1]],
                })
                .collect();
            let Some(first) = frags.first() else {
                continue;
            };
            let bytes = build_chunk(self.next_seq, &frags, false)?;
            self.next_seq += 1;
            let first_ti = self
                .tracks
                .iter()
                .position(|t| t.spec.track_id == first.track_id)
                .ok_or(Error::InvalidInput("follow: part track vanished"))?;
            let independent = first.samples.first().is_some_and(|s| s.flags.is_sync);
            let duration = span_secs(
                &self.tracks[first_ti],
                bounds[first_ti][k],
                bounds[first_ti][k + 1],
            );
            emitted_parts += 1;
            self.ready.push_back(LlHlsStageOutput::Part(PartInfo {
                bytes,
                duration,
                independent,
                segment_seq: seg_seq,
                part_index,
            }));
        }

        let splits: Vec<usize> = bounds.iter().map(|b| b[nparts]).collect();
        self.finish_follower_segment(seg_seq, &splits, emitted_parts)
    }

    /// Anchor each active track's first follower segment to the shared
    /// timeline origin: its `tfdt` starts at the first sample's decode time
    /// minus the origin, also after a silent stretch (`gap_pending`).
    fn apply_follower_origin(&mut self, active: &[bool]) {
        let Some((origin, origin_ts)) = self.timeline_origin else {
            return;
        };
        for (t, &is_active) in self.tracks.iter_mut().zip(active) {
            if !is_active || (t.origin_applied && !t.gap_pending) {
                continue;
            }
            t.origin_applied = true;
            t.gap_pending = false;
            let first = follower_dts(&t.pending[0]);
            let offset = i128::from(first) * i128::from(origin_ts)
                - i128::from(origin) * i128::from(t.spec.timescale);
            let ticks = offset / i128::from(origin_ts);
            t.seg_base_decode = u64::try_from(ticks).unwrap_or(0);
            t.part_base_decode = t.seg_base_decode;
        }
    }

    /// Emit the whole segment over `pending[..splits[track]]` of every track
    /// and drop those samples. A track with no samples in the segment is
    /// marked so its next segment re-anchors to the origin; nothing is
    /// emitted when no track has any.
    fn finish_follower_segment(
        &mut self,
        segment_seq: u32,
        splits: &[usize],
        part_count: u32,
    ) -> Result<()> {
        for (t, &split) in self.tracks.iter_mut().zip(splits) {
            if split == 0 {
                t.gap_pending = true;
            }
        }
        let Some(first_ti) = splits.iter().position(|&b| b > 0) else {
            return Ok(());
        };
        let seg_frags: Vec<FragmentTrackData<'_>> = self
            .tracks
            .iter()
            .zip(splits)
            .filter(|&(_, &b)| b > 0)
            .map(|(t, &b)| FragmentTrackData {
                track_id: t.spec.track_id,
                base_media_decode_time: t.seg_base_decode,
                samples: &t.pending[..b],
            })
            .collect();
        let bytes = crate::pipeline::build_media_segment(self.next_seq, &seg_frags)?;
        self.next_seq += 1;
        let duration = span_secs(&self.tracks[first_ti], 0, splits[first_ti]);
        self.ready.push_back(LlHlsStageOutput::Segment(SegmentInfo {
            bytes,
            duration,
            segment_seq,
            part_count,
        }));

        for (t, &split) in self.tracks.iter_mut().zip(splits) {
            if split == 0 {
                continue;
            }
            let mut clock = MediaClock::resumed_at(t.seg_clock.last_dts());
            let ticks: u64 = t.pending[..split].iter().map(|s| clock.tick(s)).sum();
            t.pending.drain(..split);
            t.seg_base_decode += ticks;
            t.part_base_decode = t.seg_base_decode;
            t.part_start = 0;
            t.seg_clock = clock;
            t.part_clock = clock;
        }
        Ok(())
    }

    /// Start recording the boundaries of every segment this segmenter cuts, so
    /// a follower can be fed the same cuts through
    /// [`take_ready_cut_plans`](Self::take_ready_cut_plans). Off by default; once
    /// on, the caller must drain the plans or they accumulate.
    pub fn record_cuts(&mut self) {
        self.record_cuts = true;
    }

    /// Align a follower's timeline with its leader's (issue #1428 slice 2b).
    ///
    /// The first segment each track emits in follower mode starts its
    /// `tfdt` at that track's first sample's decode time minus the origin
    /// (the leader's first segment start, rescaled into the track's
    /// timescale, floored at zero; a zero `origin_timescale` is ignored), rather than at zero. Later segments
    /// continue contiguously from there. A no-op for a segmenter that is not
    /// following, and never consulted by a leader.
    pub fn set_timeline_origin(&mut self, origin_dts: i64, origin_timescale: u32) {
        if origin_timescale != 0 {
            self.timeline_origin = Some((origin_dts, origin_timescale));
        }
    }

    /// Remove and return the [`CutPlan`] of every segment finished since the
    /// last call, in order. Each plan carries the segment's sequence number and
    /// the absolute decode time of its first anchor-track sample (and of each
    /// part's), in the anchor track's timescale. Empty unless
    /// [`record_cuts`](Self::record_cuts) was called.
    pub fn take_ready_cut_plans(&mut self) -> Vec<CutPlan> {
        self.ready_cuts.drain(..).collect()
    }

    /// Start handing out each part boundary as its part closes, for
    /// [`take_ready_part_cuts`](Self::take_ready_part_cuts). Independent of
    /// [`record_cuts`](Self::record_cuts); once on, the caller must drain the
    /// boundaries or they accumulate.
    pub fn record_part_cuts(&mut self) {
        self.record_part_cuts = true;
    }

    /// Remove and return every part boundary handed out since the last call, in
    /// the order the parts closed. Each carries the segment's media sequence
    /// number, the part index and the part's first-sample decode time.
    pub fn take_ready_part_cuts(&mut self) -> Vec<PartCutReady> {
        self.ready_part_cuts.drain(..).collect()
    }

    /// Remove and return every part finished since the last call, in order.
    /// Any not-yet-taken segments remain queued (retrievable via
    /// [`take_ready_segments`](Self::take_ready_segments) or `Stage::poll`),
    /// in their original relative order.
    pub fn take_ready_parts(&mut self) -> Vec<PartInfo> {
        let mut parts = Vec::new();
        let mut remaining = VecDeque::with_capacity(self.ready.len());
        for item in self.ready.drain(..) {
            match item {
                LlHlsStageOutput::Part(p) => parts.push(p),
                other => remaining.push_back(other),
            }
        }
        self.ready = remaining;
        parts
    }

    /// Remove and return every full segment finished since the last call, in
    /// order — distinct from the parts. Any not-yet-taken parts remain
    /// queued (retrievable via [`take_ready_parts`](Self::take_ready_parts)
    /// or `Stage::poll`), in their original relative order.
    pub fn take_ready_segments(&mut self) -> Vec<SegmentInfo> {
        let mut segments = Vec::new();
        let mut remaining = VecDeque::with_capacity(self.ready.len());
        for item in self.ready.drain(..) {
            match item {
                LlHlsStageOutput::Segment(s) => segments.push(s),
                other => remaining.push_back(other),
            }
        }
        self.ready = remaining;
        segments
    }

    /// Finalize the current segment: flush its remaining un-parted samples as a
    /// final part, emit the whole segment, then open the next segment.
    fn finish_segment(&mut self) -> Result<()> {
        let segment_start = {
            let t = &self.tracks[self.anchor];
            t.pending
                .first()
                .and_then(|s| s.dts)
                .unwrap_or_else(|| ticks_to_i64(t.seg_base_decode))
        };
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

        // See `part_target_secs` above for why `.max(1)` guards this division.
        let seg_duration =
            self.anchor_seg_dur as f64 / self.tracks[self.anchor].spec.timescale.max(1) as f64;
        self.ready.push_back(LlHlsStageOutput::Segment(SegmentInfo {
            bytes: seg_bytes,
            duration: seg_duration,
            segment_seq: self.current_segment,
            part_count: self.next_part_index,
        }));
        if self.record_cuts {
            let cuts = core::mem::take(&mut self.current_cuts);
            self.ready_cuts.push_back(CutPlan::new(
                self.current_segment,
                segment_start,
                self.tracks[self.anchor].spec.timescale,
                cuts,
            ));
        }

        // Advance per-track decode times past the whole segment and clear buffers.
        for t in &mut self.tracks {
            let clock = &mut t.seg_clock;
            let dur: u64 = t.pending.iter().map(|s| clock.tick(s)).sum();
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
    /// `final_part`, drain every remaining sample of every track. Otherwise the
    /// anchor drains every buffered sample and each non-anchor track drains the
    /// samples whose buffered decode span falls inside the anchor's part span —
    /// so audio accompanies the anchor in every part instead of arriving only in
    /// the segment's last one (RFC 8216bis §4.4.4.9 / audit r05-W27).
    fn emit_part(&mut self, final_part: bool) -> Result<()> {
        let anchor = self.anchor;
        let part_start = {
            let t = &self.tracks[anchor];
            t.pending
                .get(t.part_start)
                .and_then(|s| s.dts)
                .unwrap_or_else(|| ticks_to_i64(t.part_base_decode))
        };

        // The decode span this part may cover, in the *anchor's* timescale: the
        // part target, capped by what the anchor has actually buffered
        // (`anchor_part_dur`). A regular part must not exceed the part target
        // (RFC 8216bis §4.4.4.9), so the drain stops at the first sample that
        // would cross it; the final part of a segment drains everything.
        let anchor_scale = self.tracks[anchor].spec.timescale.max(1) as u64;
        let part_span = if final_part {
            u64::MAX
        } else {
            self.anchor_part_dur.min(self.part_target_ticks)
        };

        // Per track, the [part_start .. end) sample range this part drains.
        // Every track drains the prefix of its un-parted samples whose decode
        // span is within the part span, scaled to its own timescale — so a
        // lower-rate track (audio) still contributes its samples to *this* part
        // rather than waiting for the segment's final one (audit r05-W27).
        let take_ends: Vec<usize> = self
            .tracks
            .iter()
            .map(|t| {
                if final_part {
                    return t.pending.len();
                }
                let track_scale = t.spec.timescale.max(1) as u64;
                let track_span = if part_span == u64::MAX {
                    u64::MAX
                } else {
                    part_span.saturating_mul(track_scale) / anchor_scale
                };
                take_span(t, track_span)
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
            .map(|s| s.flags.is_sync)
            .unwrap_or(false);

        // Part duration = the anchor's decoded span actually drained into this
        // part (`take_ends[anchor] - part_start` samples), not the whole
        // buffered span — the samples past the part target stay for the next
        // part, so `#EXT-X-PART:DURATION` must reflect what is really here.
        // See `part_target_secs` above for why `.max(1)` guards this division.
        let anchor_span_ticks: u64 = {
            let t = &self.tracks[anchor];
            let clock = &mut MediaClock::resumed_at(t.part_clock.last_dts());
            t.pending[t.part_start..take_ends[anchor]]
                .iter()
                .map(|s| clock.tick(s))
                .sum()
        };
        let part_secs = if anchor_span_ticks > 0 {
            anchor_span_ticks as f64 / self.tracks[anchor].spec.timescale.max(1) as f64
        } else {
            // The anchor drained nothing: an earlier part already took its last
            // sample and only another track (audio, teletext) still has samples
            // to flush at the segment cut. RFC 8216bis section 4.4.4.9: the
            // final Partial Segment of a Parent Segment may be shorter than 85%
            // of the Part Target Duration, but `DURATION` is the duration of
            // the Partial Segment and a playlist must not advertise a Partial
            // Segment of zero duration, so report the span of the media it does
            // carry rather than the anchor's empty span.
            self.tracks
                .iter()
                .zip(&take_ends)
                .map(|(t, &end)| {
                    let clock = &mut MediaClock::resumed_at(t.part_clock.last_dts());
                    let ticks: u64 = t.pending[t.part_start..end]
                        .iter()
                        .map(|s| clock.tick(s))
                        .sum();
                    ticks as f64 / t.spec.timescale.max(1) as f64
                })
                .fold(0.0, f64::max)
        };

        let seq = self.next_seq;
        let part_bytes = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .zip(&take_ends)
                .filter(|&(t, &end)| end > t.part_start)
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

        // Advance per-track part cursors and part-base decode times. The
        // `[part_start..end)` ranges are contiguous and in decode order across
        // calls, so `part_clock` sees each sample exactly once.
        for (t, &end) in self.tracks.iter_mut().zip(&take_ends) {
            let clock = &mut t.part_clock;
            let mut dur: u64 = 0;
            for s in &t.pending[t.part_start..end] {
                dur += clock.tick(s);
            }
            t.part_base_decode += dur;
            t.part_start = end;
        }

        let part_index = self.next_part_index;
        self.next_part_index += 1;

        // Re-seed the accumulator with the span still buffered on the anchor.
        // `take_span` stops *before* the sample that would cross the part
        // target, so `pending` keeps that sample and its elapsed span must not
        // be discarded: resetting to 0 instead let the leftover grow by about
        // one sample per part, and the segment's final part ended up carrying
        // all of it — a part many times the part target on a long segment
        // (audit fix wave 2, item 3).
        self.anchor_part_dur = {
            let t = &self.tracks[anchor];
            let mut clock = MediaClock::resumed_at(t.part_clock.last_dts());
            t.pending[t.part_start..]
                .iter()
                .map(|s| clock.tick(s))
                .sum()
        };

        self.ready.push_back(LlHlsStageOutput::Part(PartInfo {
            bytes: part_bytes,
            duration: part_secs,
            independent,
            segment_seq: self.current_segment,
            part_index,
        }));
        if self.record_cuts {
            self.current_cuts.push(PartCut::new(
                part_index,
                part_start,
                self.tracks[anchor].spec.timescale,
            ));
        }
        if self.record_part_cuts {
            self.ready_part_cuts.push_back(PartCutReady {
                msn: self.current_segment,
                part_index,
                start_dts: part_start,
                timescale: self.tracks[anchor].spec.timescale,
            });
        }
        Ok(())
    }
}

/// [`LlHlsSegmenter`]'s [`Stage::Out`] — this segmenter drains two distinct
/// kinds of ready output today ([`take_ready_parts`](LlHlsSegmenter::take_ready_parts)
/// / [`take_ready_segments`](LlHlsSegmenter::take_ready_segments)); `Stage`
/// needs one `Out` type, and an enum is the honest way to carry that
/// distinction through rather than silently merging or dropping one kind.
/// Deliberately not unified with any other segmenter's `Out` — see the
/// `stage` module docs on why each implementor states its own shape.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LlHlsStageOutput {
    /// A finished partial segment — see [`PartInfo`].
    Part(PartInfo),
    /// A finished full segment — see [`SegmentInfo`].
    Segment(SegmentInfo),
}

/// [`Stage`] adoption (media plane step 2e-2): `In = (u32, Sample)`, same
/// reasoning as [`crate::segmenter::Segmenter`]'s impl. `Out =
/// [`LlHlsStageOutput`]`, covering both parts and segments (see that type's
/// docs).
///
/// Every inherent method — [`push`](Self::push), [`take_ready_parts`
/// ](Self::take_ready_parts), [`take_ready_segments`](Self::take_ready_segments),
/// [`flush`](Self::flush) — keeps working unchanged; this impl is an
/// additional, uniform way to drive the same engine. [`Stage::poll`] and the
/// inherent `take_ready_parts`/`take_ready_segments` drains all read from the
/// *same* `ready` queue (there is no separate staging copy) — `poll` pops the
/// raw part-or-segment item in exact production order, while each inherent
/// drain filters out its own variant and leaves the other kind queued. So a
/// part or segment is delivered exactly once no matter which API — inherent,
/// `Stage`, or a mix of both on the same instance — the caller uses to
/// retrieve it.
impl Stage for LlHlsSegmenter {
    type In<'a> = (TrackId, Sample);
    type Out = LlHlsStageOutput;
    type Error = Error;

    fn feed(&mut self, (track_id, sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
        self.push(track_id, sample)
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        // Parts/segments are only cut in reaction to `push`/`flush` — no
        // rate-scheduled or timeout work.
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// `saturated` once any track holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples — the same bound (and reasoning) as
    /// [`Segmenter`](crate::segmenter::Segmenter)'s: past it
    /// [`feed`](Stage::feed) errors rather than buffering a stream that never
    /// produces the sync sample a segment must open on.
    ///
    /// This segmenter is the one with the sharpest need for the bound: past
    /// the segment target it *also* stops emitting parts (a part is only
    /// flushed while `anchor_seg_dur < target_ticks`, so the segment's tail
    /// is held for the boundary), so a single-IDR stream produces no part
    /// **and** no segment while `pending` grows.
    fn demand(&self) -> Demand {
        if self
            .tracks
            .iter()
            .any(|t| t.pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK)
        {
            Demand::saturated()
        } else {
            Demand::default()
        }
    }
}
/// How many of `t`'s un-parted samples (`pending[part_start..]`) fit inside a
/// decode span of `target_ticks` (in this track's timescale).
///
/// Walks the samples with a [`MediaClock`] resumed at the last drained sample's
/// `dts`, so the duration-then-dts-delta rule matches the drain accounting in
/// [`LlHlsSegmenter::emit_part`] exactly, and stops *before* the sample that
/// crosses `target_ticks` (a part never over-runs the anchor's part span by a
/// whole sample; the overhang lands in the next part). Always takes at least one
/// sample when one is available, so an unusually long sample cannot stall the
/// drain forever.
fn take_span(t: &TrackState, target_ticks: u64) -> usize {
    if t.part_start >= t.pending.len() {
        return t.part_start;
    }
    let mut clock = MediaClock::resumed_at(t.part_clock.last_dts());
    let mut span: u64 = 0;
    let mut end = t.part_start;
    for s in &t.pending[t.part_start..] {
        let next = clock.tick(s);
        // Stop as soon as adding this sample would take the part *past* the
        // target: the part must stay ≤ the target (RFC 8216bis §4.4.4.9), and
        // taking the sample that reaches it would round every part up by one
        // sample (a 334 ms target with 33.3 ms AUs gave 367 ms parts).
        if end > t.part_start && span + next > target_ticks {
            break;
        }
        span += next;
        end += 1;
    }
    // No forced-take branch is needed: the `end > t.part_start` guard in the
    // loop never breaks before the first sample, so a single sample longer than
    // the part target is still carried as its own part and the drain advances.
    end
}
fn ticks_to_i64(ticks: u64) -> i64 {
    i64::try_from(ticks).unwrap_or(i64::MAX)
}

/// `value` ticks of `from_ts` as ticks of `to_ts`, rounded up, saturating at
/// the `i64` range. Both timescales must be non-zero.
fn ceil_rescale(value: i64, from_ts: u32, to_ts: u32) -> i64 {
    let numerator = i128::from(value) * i128::from(to_ts);
    let from = i128::from(from_ts);
    let floor = numerator.div_euclid(from);
    let ceil = if numerator.rem_euclid(from) == 0 {
        floor
    } else {
        floor + 1
    };
    i64::try_from(ceil).unwrap_or(if ceil < 0 { i64::MIN } else { i64::MAX })
}

/// Exact comparison of `a / a_ts` with `b / b_ts` seconds; timescales non-zero.
fn cmp_time(a: i64, a_ts: u32, b: i64, b_ts: u32) -> core::cmp::Ordering {
    (i128::from(a) * i128::from(b_ts)).cmp(&(i128::from(b) * i128::from(a_ts)))
}

/// Decode time a follower segmenter cuts on; `push` rejects samples without one.
fn follower_dts(sample: &Sample) -> i64 {
    sample.dts.unwrap_or(i64::MIN)
}

/// Duration in seconds of `track.pending[from..to]`.
fn span_secs(track: &TrackState, from: usize, to: usize) -> f64 {
    let mut clock = MediaClock::resumed_at(
        from.checked_sub(1)
            .and_then(|i| track.pending.get(i))
            .and_then(|s| s.dts)
            .or(track.seg_clock.last_dts()),
    );
    let ticks: u64 = track.pending[from..to].iter().map(|s| clock.tick(s)).sum();
    ticks as f64 / f64::from(track.spec.timescale.max(1))
}
