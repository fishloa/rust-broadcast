//! Stateful CMAF segmenter — a streaming wrapper over [`build_init_segment`] and
//! [`build_media_segment`].
//!
//! [`build_media_segment`] is a *batch* box
//! builder: hand it the samples for one segment and it emits the `styp`/`moof`/
//! `mdat`. It has no notion of *when* a segment should end. A live remuxer needs
//! that decision: accumulate coded access units, cut a segment on a keyframe once
//! it has reached a target duration, and expose finished segments to the caller.
//!
//! [`Segmenter`] adds exactly that state machine:
//!
//! - [`Segmenter::init_segment`] — the `ftyp`+`moov` init, available immediately.
//! - [`Segmenter::push`] — feed one [`Sample`] for a track, in decode order.
//! - [`Segmenter::take_ready`] — drain media segments finished so far.
//! - [`Segmenter::flush`] — finalize the trailing partial segment at end-of-stream.
//! - [`Segmenter::mark_discontinuity`] — mark the *next* cut as discontinuous
//!   (RFC 8216 §4.3.4.3).
//! - [`Segmenter::take_ready_with_meta`] — like `take_ready` but also returns
//!   per-segment [`SegmentMeta`] that carries the discontinuity flag for HLS
//!   playlist assembly.
//!
//! Segments are cut on the **anchor track** (the first video track, or the first
//! track if audio-only): when a sync sample arrives *and* the anchor's buffered
//! duration has reached the target, the buffered samples across all tracks are
//! emitted as one media segment and the incoming keyframe starts the next one. So
//! every video segment begins on a random-access point, as CMAF requires, and no
//! sample is dropped or reordered — the concatenation of all segments carries the
//! full input stream with contiguous per-track decode times.
//!
//! # Discontinuity detection
//!
//! A media-timeline discontinuity (RFC 8216 §4.3.4.3) is signalled in two ways:
//!
//! 1. **Explicit**: call [`Segmenter::mark_discontinuity`] before the next
//!    [`Segmenter::push`] call that triggers a segment cut. The *next* segment
//!    that is cut will be marked discontinuous.
//!
//! 2. **Auto-detect**: when the init segment bytes change between two consecutive
//!    cuts (e.g. because the codec config, `EXT-X-MAP`, or track layout changed),
//!    the segmenter automatically marks the later segment as discontinuous.
//!
//! Both mechanisms set the [`SegmentMeta::discontinuous`] flag returned by
//! [`Segmenter::take_ready_with_meta`], which callers can forward directly to
//! [`crate::hls::MediaSegment::discontinuous`].

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::pipeline::{
    CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment, build_media_segment,
};

/// Per-segment metadata returned alongside the media segment bytes by
/// [`Segmenter::take_ready_with_meta`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentMeta {
    /// `true` when this segment is a media-timeline discontinuity — the
    /// caller should emit `#EXT-X-DISCONTINUITY` (RFC 8216 §4.3.4.3)
    /// immediately before this segment's `#EXTINF` line in the HLS playlist.
    ///
    /// Set either by [`Segmenter::mark_discontinuity`] (explicit) or
    /// automatically when the init segment bytes differ from those of the
    /// preceding cut (init-change auto-detect).
    pub discontinuous: bool,
}

/// Per-track accumulation state for the segment currently being built.
struct TrackState {
    spec: TrackSpec,
    /// Samples buffered for the current (not-yet-cut) segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of the first *pending* sample = sum of the durations of every
    /// sample already emitted in earlier segments (media-timescale ticks). This is
    /// the `base_media_decode_time` (`tfdt`) of the next segment for this track.
    base_decode: u64,
}

/// A stateful CMAF segmenter. Build it from the same [`TrackSpec`]s used for the
/// init segment, `push` coded samples in decode order, and pull finished media
/// segments with `take_ready`; `flush` emits the final partial segment.
///
/// ```
/// use transmux::{CodecConfig, Sample, Segmenter, TrackSpec};
/// # use transmux::AVCConfigurationBox;
/// # fn spec() -> TrackSpec { unimplemented!() }
/// # fn au(sync: bool) -> Sample { Sample::from_raw(vec![0u8; 4], 3000) }
/// # if false {
/// let mut seg = Segmenter::new(vec![spec()], 1000, 2.0).unwrap();
/// let init = seg.init_segment().unwrap();      // ftyp + moov
/// seg.push(1, au(true)).unwrap();              // keyframe
/// seg.push(1, au(false)).unwrap();
/// for media in seg.take_ready() { /* write out */ }
/// seg.flush().unwrap();                        // trailing segment
/// # }
/// ```
pub struct Segmenter {
    tracks: Vec<TrackState>,
    movie_timescale: u32,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Buffered duration of the anchor's `pending` samples (media-timescale ticks).
    anchor_pending_dur: u64,
    /// `sequence_number` of the next media segment (`moof` `mfhd`), 1-based.
    next_seq: u32,
    /// Media segments finished but not yet taken by the caller (bytes + meta).
    ready: Vec<(Vec<u8>, SegmentMeta)>,
    /// Explicit discontinuity: when `true` the *next* cut is marked discontinuous.
    /// Reset to `false` after each cut.
    pending_discontinuity: bool,
    /// The init-segment bytes from the last cut (or the initial build), used to
    /// auto-detect init changes.  `None` before the first segment is cut.
    last_init: Option<Vec<u8>>,
}

impl Segmenter {
    /// Create a segmenter for `tracks`, cutting segments roughly every
    /// `target_duration_secs` seconds on the anchor track's keyframes.
    ///
    /// The anchor is the first video track (falling back to the first track for
    /// audio-only). `movie_timescale` matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s, or
    /// `target_duration_secs` is not positive and finite.
    pub fn new(
        tracks: Vec<TrackSpec>,
        movie_timescale: u32,
        target_duration_secs: f64,
    ) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput("segmenter needs at least one track"));
        }
        if !(target_duration_secs.is_finite() && target_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "target_duration_secs must be positive and finite",
            ));
        }
        // Reject duplicate track IDs (they would collide in the moof/moov).
        for (i, a) in tracks.iter().enumerate() {
            if tracks[i + 1..].iter().any(|b| b.track_id == a.track_id) {
                return Err(Error::InvalidInput("duplicate track_id"));
            }
        }

        // Anchor = first video track; else first track (audio-only).
        let anchor = tracks
            .iter()
            .position(|t| matches!(t.config, CodecConfig::Avc { .. }))
            .unwrap_or(0);

        let anchor_timescale = tracks[anchor].timescale as f64;
        let target_ticks = (target_duration_secs * anchor_timescale) as u64;
        let target_ticks = target_ticks.max(1); // never a zero-length target

        let tracks = tracks
            .into_iter()
            .map(|spec| TrackState {
                spec,
                pending: Vec::new(),
                base_decode: 0,
            })
            .collect();

        Ok(Self {
            tracks,
            movie_timescale,
            anchor,
            target_ticks,
            anchor_pending_dur: 0,
            next_seq: 1,
            ready: Vec::new(),
            pending_discontinuity: false,
            last_init: None,
        })
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`). Stable for the
    /// life of the segmenter; write it once before any media segment.
    pub fn init_segment(&self) -> Result<Vec<u8>> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        build_init_segment(&specs, self.movie_timescale)
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// If this is a sync sample on the anchor track and the anchor has already
    /// buffered at least the target duration, the buffered samples are cut into a
    /// media segment (retrievable via [`take_ready`](Self::take_ready)) *before*
    /// this sample is buffered — so the new sample opens the next segment on a
    /// random-access point.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, or the underlying
    /// [`build_media_segment`] fails while cutting.
    pub fn push(&mut self, track_id: u32, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        // Cut before buffering when the anchor hits a keyframe past the target.
        if idx == self.anchor
            && sample.is_sync
            && self.anchor_pending_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty()
        {
            self.cut_segment()?;
        }

        if idx == self.anchor {
            self.anchor_pending_dur += sample.duration as u64;
        }
        self.tracks[idx].pending.push(sample);
        Ok(())
    }

    /// Finalize the trailing partial segment (call once at end-of-stream). A
    /// no-op if nothing is buffered. The emitted segment, if any, is appended to
    /// the ready queue — retrieve it with [`take_ready`](Self::take_ready).
    ///
    /// # Errors
    /// Propagates a [`build_media_segment`] failure.
    pub fn flush(&mut self) -> Result<()> {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.cut_segment()?;
        }
        Ok(())
    }

    /// Mark the *next* segment cut as a media-timeline discontinuity
    /// (RFC 8216 §4.3.4.3). The flag is consumed at the next segment boundary
    /// and reset; call this again before each discontinuous cut.
    pub fn mark_discontinuity(&mut self) {
        self.pending_discontinuity = true;
    }

    /// Remove and return every media segment finished since the last call, in
    /// order. Each element is a complete `styp`+`moof`+`mdat` segment.
    ///
    /// Use [`take_ready_with_meta`](Self::take_ready_with_meta) to also
    /// retrieve per-segment metadata (including the discontinuity flag).
    pub fn take_ready(&mut self) -> Vec<Vec<u8>> {
        self.ready.drain(..).map(|(bytes, _meta)| bytes).collect()
    }

    /// Remove and return every media segment finished since the last call,
    /// together with their [`SegmentMeta`]. The segments are in playlist order.
    ///
    /// The [`SegmentMeta::discontinuous`] flag indicates whether
    /// `#EXT-X-DISCONTINUITY` should precede this segment's `#EXTINF` line.
    pub fn take_ready_with_meta(&mut self) -> Vec<(Vec<u8>, SegmentMeta)> {
        core::mem::take(&mut self.ready)
    }

    /// Cut the buffered samples across all tracks into one media segment, advance
    /// each track's `base_decode`, and clear the buffers.
    fn cut_segment(&mut self) -> Result<()> {
        let seg = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .filter(|t| !t.pending.is_empty())
                .map(|t| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.base_decode,
                    samples: &t.pending,
                })
                .collect();
            if frags.is_empty() {
                return Ok(());
            }
            build_media_segment(self.next_seq, &frags)?
        }; // immutable borrow of `self.tracks` ends here, before the mutation below

        // Determine the discontinuity flag for this segment:
        // - explicit (`mark_discontinuity` was called), OR
        // - auto-detect: init bytes differ from those of the previous cut.
        let current_init = build_init_segment(
            &self
                .tracks
                .iter()
                .map(|t| t.spec.clone())
                .collect::<Vec<_>>(),
            self.movie_timescale,
        )?;
        let init_changed = self
            .last_init
            .as_ref()
            .map(|prev| prev != &current_init)
            .unwrap_or(false); // first segment: no previous init to compare
        let discontinuous = self.pending_discontinuity || init_changed;
        self.last_init = Some(current_init);
        self.pending_discontinuity = false;

        self.next_seq += 1;
        for t in &mut self.tracks {
            let dur: u64 = t.pending.iter().map(|s| s.duration as u64).sum();
            t.base_decode += dur;
            t.pending.clear();
        }
        self.anchor_pending_dur = 0;
        self.ready.push((seg, SegmentMeta { discontinuous }));
        Ok(())
    }
}
