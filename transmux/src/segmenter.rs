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
//! Segments are cut on the **anchor track** (the first video track, else the
//! first [`is_anchor_capable`] track): when a sync sample arrives *and* the
//! anchor's buffered duration has reached the target, the buffered samples
//! across all tracks are emitted as one media segment and the incoming keyframe
//! starts the next one. So every video segment begins on a random-access point,
//! as CMAF requires, and no sample is dropped or reordered — the concatenation
//! of all segments carries the full input stream with contiguous per-track
//! decode times.
//!
//! # Anchor progress and the un-cut bound
//!
//! "The anchor's buffered duration" is accumulated by [`MediaClock`]: each
//! anchor sample's own `duration` when that is a real, non-zero span, and
//! otherwise the **`dts` delta** from the previous anchor sample. `dts` is
//! absolute (media plane step 2c), so elapsed media time is derivable without
//! `duration` at all — and it must be, because a `duration` of `Some(0)` is
//! routine on live input and would otherwise freeze the accumulator so that no
//! segment is ever cut and the pending buffer grows without bound.
//!
//! A stream that never produces a second sync sample (single-IDR / infinite
//! GOP — legal, and common for screen capture) can still never be cut, because
//! cutting mid-GOP would break CMAF's random-access guarantee. That case is
//! bounded rather than silently mis-cut: past
//! [`MAX_PENDING_SAMPLES_PER_TRACK`] un-cut samples,
//! [`Stage::demand`] reports `saturated` and
//! [`push`](Segmenter::push) returns a named error. These same three
//! primitives are shared verbatim by the other three segmenters.
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

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::{Demand, Stage, Timestamp};

use crate::error::{Error, Result};
use crate::pipeline::{
    CodecConfig, DataCarriage, FragmentTrackData, Sample, TrackSpec, build_init_segment,
    build_media_segment,
};

// ── Shared segmentation primitives (all four segmenters) ────────────────────
//
// [`Segmenter`] (CMAF), [`LlSegmenter`](crate::ll_dash::LlSegmenter)
// (chunked CMAF / LL-DASH), [`LlHlsSegmenter`](crate::ll_hls::LlHlsSegmenter)
// (LL-HLS parts) and [`StreamingTsHlsSegmenter`](crate::ts_hls::StreamingTsHlsSegmenter)
// (classic TS-HLS) all cut on the same rule — "the anchor track's next sync
// sample once the anchor has buffered the target duration" — so anchor
// selection, anchor-clock accounting, and the un-cut buffer bound live here
// once and are shared, rather than re-derived (and drifting) per module.

/// True when `config`'s samples can advance an anchor clock, so the track is
/// eligible to be the segmentation anchor (the keyframe-cut boundary track).
///
/// False only for a section-carried [`CodecConfig::Data`] track
/// ([`DataCarriage::Sections`]): ISO/IEC 13818-1 §2.4.4 PSI/private sections
/// carry no PES timestamp at all, so `TsDemux`/`StreamingTsDemux` never
/// fabricate a `dts` *or* a `duration` for them (media plane step 2c) — every
/// sample is `dts: None, duration: None`, so neither term of
/// [`MediaClock::tick`] can ever advance and the segmenter would buffer
/// forever without cutting. A PES-carried `CodecConfig::Data` track
/// ([`DataCarriage::Pes`], e.g. private PES data) *does* get a real
/// lookahead-derived duration and an absolute dts exactly like audio/video,
/// so it stays anchor-eligible.
///
/// Deliberately narrower than [`CodecConfig::is_muxable_in_bmff`], which
/// excludes every `Data` track (`Pes` or `Sections`) plus `Subtitle`: unlike
/// ISOBMFF, this crate's TS mux path ([`crate::ts_mux`]) *can* and does carry
/// section tracks verbatim (raw on their own PID rather than PES), so a
/// section-carried track must stay in the segmenter's track set to be muxed
/// — it is just never chosen as the anchor.
pub(crate) fn is_anchor_capable(config: &CodecConfig) -> bool {
    !matches!(
        config,
        CodecConfig::Data {
            carriage: DataCarriage::Sections,
            ..
        }
    )
}

/// Choose the anchor track index used for segment-cut boundaries: the first
/// video track (any [`CodecConfig::is_video`] codec — issue #628), else the
/// first [`is_anchor_capable`] track.
///
/// Never silently falls back to track 0 when no track qualifies: that would
/// pick a track whose clock can never advance (e.g. a section-only,
/// video-less input carrying only a SCTE-35 splice-info track), and the
/// segmenter would then buffer forever without ever cutting a segment. That
/// case is a construction error instead.
///
/// # Errors
/// [`Error::InvalidInput`] if `configs` is empty, or no track is
/// anchor-capable (every track is a section-carried [`CodecConfig::Data`]
/// track).
pub(crate) fn choose_anchor<'a, I>(configs: I) -> Result<usize>
where
    I: Iterator<Item = &'a CodecConfig>,
{
    let configs: Vec<&CodecConfig> = configs.collect();
    configs
        .iter()
        .position(|c| c.is_video())
        .or_else(|| configs.iter().position(|c| is_anchor_capable(c)))
        .ok_or(Error::InvalidInput(
            "no anchor-capable track: segmentation needs a video track, or at least one \
             track whose samples carry a real duration or decode time — a track set of \
             only section-carried data tracks can never advance a keyframe-cut boundary",
        ))
}

/// Per-track elapsed-media accounting: turns a stream of [`Sample`]s into
/// per-sample tick increments in that track's media timescale.
///
/// The increment for a sample is its own `duration` when that is a real,
/// non-zero span; **otherwise the `dts` delta from the previous sample of the
/// same track**. Media plane step 2c made `Sample::dts` an *absolute* tick
/// value, so elapsed media time is derivable without `duration` at all — and
/// it has to be, because `duration` is legitimately `Some(0)` on real inputs:
/// [`StreamingFlvDemux`](crate::flv_stream::StreamingFlvDemux) derives it as
/// the forward delta between consecutive FLV tag timestamps, so the first
/// sample of an RTMP publish, and any two tags sharing a timestamp, yield `0`.
/// Before this fallback existed a `Some(0)`/`None` duration froze every
/// segmenter's anchor accumulator: no segment was ever cut and the pending
/// buffer grew without bound.
///
/// `duration` stays the *primary* term so a stream that carries real
/// durations segments byte-identically to before this fallback existed; the
/// dts delta only fills the gap `duration` leaves. When neither is available
/// the increment is `0` — a track in that state is not
/// [`is_anchor_capable`], and the anchor role is refused at construction.
#[derive(Debug, Default)]
pub(crate) struct MediaClock {
    /// `dts` of the most recent sample that carried one — deliberately *not*
    /// reset at a segment/part boundary, so the first sample of a new window
    /// can still take its delta from the last sample of the previous one.
    last_dts: Option<i64>,
}

impl MediaClock {
    /// A clock that has not yet seen a sample.
    pub(crate) const fn new() -> Self {
        Self { last_dts: None }
    }

    /// The elapsed-tick increment `sample` contributes, and record its `dts`
    /// as the baseline for the next call. See the type docs for the
    /// duration-then-dts-delta rule.
    pub(crate) fn tick(&mut self, sample: &Sample) -> u64 {
        let increment = match sample.duration {
            Some(duration) if duration > 0 => u64::from(duration),
            _ => match (self.last_dts, sample.dts) {
                (Some(previous), Some(now)) => now.saturating_sub(previous).max(0) as u64,
                _ => 0,
            },
        };
        if sample.dts.is_some() {
            self.last_dts = sample.dts;
        }
        increment
    }
}

/// The longest run of un-cut anchor media a segmenter will buffer while
/// waiting for the sync sample it needs to open the next segment on, in
/// seconds. Ten times RFC 8216 §4.3.3.1's recommended 6-second target
/// duration (DASH-IF LL IOP targets 1–4 s), so no conformant configuration
/// can reach it.
const MAX_UNCUT_SECS: usize = 60;

/// Ceiling on an anchor track's sample rate, in samples/second: 120 fps is
/// the top video frame rate in ITU-R BT.2100 / ATSC A/341, and every audio
/// anchor is far slower (48 kHz AAC is 46.9 frames/s at 1024 samples/frame).
const MAX_ANCHOR_RATE_HZ: usize = 120;

/// Hard bound on the number of samples any one track may hold **un-cut**
/// inside a segmenter (issue: single-IDR / infinite-GOP stall).
///
/// A stream with one keyframe at the start and none after — legal, and
/// routine for screen capture and low-motion surveillance — never satisfies
/// the "next sync sample" half of the cut rule, so without a bound every
/// segmenter buffers until memory is exhausted while
/// [`Stage::demand`](broadcast_common::Stage::demand) still answers "not
/// saturated", inviting a well-behaved driver to keep feeding.
///
/// Cutting mid-GOP is *not* the answer: a CMAF segment (and a classic-HLS
/// `.ts` segment) must begin on a random-access point, so a segment cut on a
/// non-sync sample would be non-conformant. Instead the bound is on data:
/// past it `demand()` reports `saturated` (the load-bearing half — a
/// cooperative driver stops feeding) and `push`/`feed` returns a named error
/// rather than growing further. It is expressed in **samples**, not bytes,
/// because the pathology is "how many access units without a random-access
/// point", which is codec-bitrate-independent; a byte bound would trip at
/// wildly different GOP lengths for a 200 kbit/s and a 200 Mbit/s stream.
/// The bound is deliberately *not* a wall-clock timeout: these types are
/// sans-IO and `no_std`.
///
/// A segmenter that has hit the bound is not wedged: `flush`/`finish` cuts
/// the whole pending buffer (a trailing partial segment is allowed not to
/// start on a keyframe) and the segmenter accepts input again.
pub(crate) const MAX_PENDING_SAMPLES_PER_TRACK: usize = MAX_UNCUT_SECS * MAX_ANCHOR_RATE_HZ;

/// The error every segmenter returns when a `push`/`feed` would grow a
/// track's un-cut buffer past [`MAX_PENDING_SAMPLES_PER_TRACK`] — i.e. the
/// anchor track produced that many samples with no sync sample to cut on.
pub(crate) fn no_sync_sample_error() -> Error {
    Error::InvalidInput(
        "segmenter pending buffer full: no anchor-track sync sample within \
         MAX_PENDING_SAMPLES_PER_TRACK samples, and a segment cannot be cut on a \
         non-sync sample without breaking the format's random-access guarantee — \
         flush() to close a trailing partial segment, or feed a keyframe",
    )
}

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
/// # fn au(sync: bool) -> Sample {
/// #     use std::sync::atomic::{AtomicI64, Ordering};
/// #     static NEXT_DTS: AtomicI64 = AtomicI64::new(0);
/// #     let dts = NEXT_DTS.fetch_add(1000, Ordering::Relaxed);
/// #     Sample::new(vec![0u8; 4], Some(dts), Some(dts), Some(1000), sync)
/// # }
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
    /// Anchor-progress clock: advances `anchor_pending_dur` from each anchor
    /// sample's `duration`, or from its `dts` delta when `duration` is absent
    /// or zero (see [`MediaClock`]).
    anchor_clock: MediaClock,
    /// `sequence_number` of the next media segment (`moof` `mfhd`), 1-based.
    next_seq: u32,
    /// Media segments finished but not yet taken by the caller (bytes + meta).
    /// The single source of truth for both the inherent `take_ready`/
    /// `take_ready_with_meta` drains and [`Stage::poll`] — whichever API the
    /// caller uses pops from this same queue, so output is delivered exactly
    /// once no matter which (or both) APIs drive this segmenter.
    ready: VecDeque<(Vec<u8>, SegmentMeta)>,
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
    /// The anchor is the first video track, falling back to the first
    /// [`is_anchor_capable`] track (audio-only input).
    /// `movie_timescale` matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration_secs` is not positive and finite, or no track is
    /// anchor-capable (every track is section-carried, so no cut boundary could
    /// ever advance — see [`choose_anchor`]).
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

        // MUX = strict but filterable (media plane step-2 fix wave 1,
        // B2-B4): a track `build_init_segment` cannot place into an ISOBMFF
        // `trak` (opaque `CodecConfig::Data` or `CodecConfig::Subtitle`) is
        // no longer silently omitted here — it surfaces the same named
        // error every other mux entry point does, the first time
        // `init_segment`/a cut actually needs the `trak`. The caller must
        // pre-filter first (e.g. with
        // `tracks.retain(|t| t.config.is_muxable_in_bmff())`) if it wants
        // to drop such tracks rather than fail.

        // Anchor = first video track (any `CodecConfig::is_video` codec —
        // issue #628); else the first anchor-capable track. Never a silent
        // fallback to track 0: a section-only track set cannot advance a cut
        // boundary at all and is refused here rather than stalling later.
        let anchor = choose_anchor(tracks.iter().map(|t| &t.config))?;

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
            anchor_clock: MediaClock::new(),
            next_seq: 1,
            ready: VecDeque::new(),
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
    /// Anchor progress is measured by [`MediaClock`]: each anchor sample's own
    /// `duration` when that is a real, non-zero span, else the `dts` delta from
    /// the previous anchor sample. `duration` alone is not enough — a
    /// `Some(0)` duration is routine on live input (see [`MediaClock`]) and
    /// would otherwise freeze the accumulator so no segment is ever cut.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, the underlying
    /// [`build_media_segment`] fails while cutting, or this track already holds
    /// [`MAX_PENDING_SAMPLES_PER_TRACK`] un-cut samples (no anchor sync sample
    /// to cut on — call [`flush`](Self::flush) to close a trailing partial
    /// segment).
    pub fn push(&mut self, track_id: u32, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        // Cut before buffering when the anchor hits a keyframe past the target.
        let cut_now = idx == self.anchor
            && sample.flags.is_sync
            && self.anchor_pending_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty();
        // `cut_segment` mutates nothing on failure (both builders it calls
        // return before any state change), so a failed cut — reachable since
        // `new` no longer filters BMFF-unmuxable tracks — must not also swallow
        // the sample that triggered it: buffer it below, then surface the
        // error. Dropping it here would silently punch a hole in the timeline
        // on every subsequent anchor keyframe.
        let cut_result = if cut_now { self.cut_segment() } else { Ok(()) };

        if self.tracks[idx].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
            return Err(no_sync_sample_error());
        }

        if idx == self.anchor {
            self.anchor_pending_dur += self.anchor_clock.tick(&sample);
        }
        self.tracks[idx].pending.push(sample);
        cut_result
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
        self.ready.drain(..).collect()
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
            let dur: u64 = t
                .pending
                .iter()
                .map(|s| s.duration.unwrap_or(0) as u64)
                .sum();
            t.base_decode += dur;
            t.pending.clear();
        }
        self.anchor_pending_dur = 0;
        self.ready.push_back((seg, SegmentMeta { discontinuous }));
        Ok(())
    }
}

/// [`Stage`] adoption (media plane step 2e-2): `In = (u32, Sample)`, the
/// segmenter's real per-call input (a track id plus one coded sample) — not
/// the byte-stream family's `&[u8]`, which would have no honest encoding of a
/// `Sample` (see the `stage` module docs). `Out` is
/// [`take_ready_with_meta`](Self::take_ready_with_meta)'s item type: bytes
/// plus the discontinuity metadata a caller needs for the HLS playlist, not a
/// bare `Vec<u8>` that would silently drop that flag.
///
/// Every inherent method — [`push`](Self::push), [`take_ready`](Self::take_ready),
/// [`take_ready_with_meta`](Self::take_ready_with_meta), [`flush`](Self::flush),
/// [`mark_discontinuity`](Self::mark_discontinuity) — keeps working unchanged;
/// this impl is an additional, uniform way to drive the same engine.
/// [`Stage::poll`] and the inherent drains all read from the *same* `ready`
/// queue (there is no separate staging copy), so a segment is delivered
/// exactly once no matter which API — inherent, `Stage`, or a mix of both on
/// the same instance — the caller uses to retrieve it.
impl Stage for Segmenter {
    type In<'a> = (u32, Sample);
    type Out = (Vec<u8>, SegmentMeta);
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
        // Segments are only cut in reaction to `push` (a keyframe past the
        // target duration) or `flush` — no rate-scheduled or timeout work.
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// `saturated` once any track holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples — the point past which [`feed`](Stage::feed) errors
    /// rather than buffering further, so a cooperative driver must stop
    /// feeding (and [`finish`](Stage::finish) to close the trailing partial
    /// segment) instead of growing this segmenter without bound waiting for a
    /// sync sample that may never arrive. Below the bound there is no hard cap
    /// to report, so `want_bytes` stays the honest "no preference" default.
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
