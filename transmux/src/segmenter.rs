//! Stateful CMAF segmenter — a streaming wrapper over [`build_init_segment`] and
//! [`build_media_segment`].
//!
//! [`build_media_segment`] is a *batch* box
//! builder: hand it the samples for one segment and it emits the `styp`/`moof`/
//! `mdat`. It has no notion of *when* a segment should end. A live remuxer needs
//! that decision: accumulate coded access units, cut a segment on a keyframe once
//! it has reached a target duration, and expose finished segments to the caller.
//!
//! [`Segmenter`] adds exactly that state machine, as a [`Stage`] over
//! [`MuxInput`]:
//!
//! - [`Segmenter::init_segment`] — the `ftyp`+`moov` init, available immediately.
//! - [`MuxInput::Sample`] — feed one [`Sample`] for a track, in decode order.
//! - [`Stage::poll`] — drain each finished segment as a [`MuxOutput::Segment`],
//!   whose [`SegmentSummary`] carries the
//!   discontinuity flag for HLS playlist assembly.
//! - [`MuxInput::Flush`] — finalize the trailing partial segment at end-of-stream.
//! - [`MuxInput::Discontinuity`] — mark the *next* cut as discontinuous
//!   (RFC 8216 §4.3.2.3).
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
//! [`Stage::demand`] reports `saturated` and [`Stage::feed`] returns a named
//! error. These same three primitives are shared verbatim by the other three
//! segmenters.
//!
//! # Discontinuity detection
//!
//! A media-timeline discontinuity (RFC 8216 §4.3.2.3) is signalled explicitly:
//! send [`MuxInput::Discontinuity`] before the next sample that triggers a
//! segment cut. The *next* segment that is cut will be marked discontinuous.
//! (A `Segmenter` cannot change its tracks' codec configuration after
//! construction, so there is no init-change to detect; a caller that starts a
//! new `Segmenter` for a new configuration sets the discontinuity itself.)
//!
//! The mechanism sets the `discontinuous` field of the
//! [`SegmentSummary`] on the emitted
//! [`MuxOutput::Segment`], which callers can forward directly to
//! [`broadcast_hls::MediaSegment::discontinuous`].

use crate::TrackId;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::{Demand, Stage, Timestamp};

use crate::error::{Error, Result};
use crate::mux_stage::{MuxInput, MuxOutput, SegmentOut, SegmentSummary};
use crate::pipeline::{
    CodecConfig, DataCarriage, DataIdentity, FragmentTrackData, Sample, TrackSpec,
    build_init_segment, build_media_segment,
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
pub fn is_anchor_capable(config: &CodecConfig) -> bool {
    !matches!(
        config,
        CodecConfig::Data {
            identity: DataIdentity::Ts {
                carriage: DataCarriage::Sections,
                ..
            },
        }
    )
}

/// Choose the anchor track index used for segment-cut boundaries: the first
/// video track (any `CodecConfig::is_video` codec — issue #628; that predicate
/// is crate-internal), else the
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
pub fn choose_anchor<'a, I>(configs: I) -> Result<usize>
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
#[derive(Debug, Default, Clone, Copy)]
pub struct MediaClock {
    /// `dts` of the most recent sample that carried one — deliberately *not*
    /// reset at a segment/part boundary, so the first sample of a new window
    /// can still take its delta from the last sample of the previous one.
    last_dts: Option<i64>,
}

impl MediaClock {
    /// A clock that has not yet seen a sample.
    pub const fn new() -> Self {
        Self { last_dts: None }
    }

    /// A clock resumed at `last_dts` — the `dts` of the last sample some
    /// *other* pass over the same track already consumed. Lets a read-only
    /// scan (e.g. deciding a segment split point without draining) take its
    /// first dts delta from the correct baseline instead of losing it.
    pub const fn resumed_at(last_dts: Option<i64>) -> Self {
        Self { last_dts }
    }

    /// The `dts` baseline the next [`tick`](Self::tick) would measure from.
    pub const fn last_dts(&self) -> Option<i64> {
        self.last_dts
    }

    /// The elapsed-tick increment `sample` contributes, and record its `dts`
    /// as the baseline for the next call. See the type docs for the
    /// duration-then-dts-delta rule.
    pub fn tick(&mut self, sample: &Sample) -> u64 {
        let increment = match sample.timing.and_then(|t| t.duration()) {
            Some(duration) if duration > 0 => u64::from(duration),
            _ => match (self.last_dts, sample.timing.and_then(|t| t.dts())) {
                (Some(previous), Some(now)) => now.saturating_sub(previous).max(0) as u64,
                _ => 0,
            },
        };
        if sample.timing.and_then(|t| t.dts()).is_some() {
            self.last_dts = sample.timing.and_then(|t| t.dts());
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
/// [`Stage::demand`] still answers "not
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
pub const MAX_PENDING_SAMPLES_PER_TRACK: usize = MAX_UNCUT_SECS * MAX_ANCHOR_RATE_HZ;

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

/// Per-track accumulation state for the segment currently being built.
struct TrackState {
    spec: TrackSpec,
    /// Samples buffered for the current (not-yet-cut) segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of the first *pending* sample = the elapsed media time of every
    /// sample already emitted in earlier segments (media-timescale ticks). This is
    /// the `base_media_decode_time` (`tfdt`) of the next segment for this track.
    base_decode: u64,
    /// Advances `base_decode` past each sample as it is flushed into a
    /// segment. A *separate* [`MediaClock`] from the anchor's: this one walks
    /// each track's samples once at cut time, the anchor's walks the anchor's
    /// once at push time, and each needs its own `last_dts` baseline. Without
    /// it `base_decode` would be a plain `duration` sum and would stay at `0`
    /// forever on a `duration: Some(0)` stream, so every segment's `tfdt`
    /// would collapse to the same value (see [`MediaClock`]).
    flush_clock: MediaClock,
}

/// A stateful CMAF segmenter. Build it from the same [`TrackSpec`]s used for the
/// init segment, feed coded samples in decode order, and poll finished media
/// segments; a [`MuxInput::Flush`] closes the final partial segment.
///
/// ```
/// use transmux::{CodecConfig, Sample, Segmenter, TrackSpec};
/// # use isobmff::{AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps};
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
/// #     TrackSpec::new(transmux::TrackId::new(1), broadcast_core::timescale::Timescale::try_new(90_000).expect("non-zero timescale"), CodecConfig::Avc {
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
/// #         .expect("valid timing")
/// # }
/// let mut seg = Segmenter::new(vec![spec()], broadcast_core::timescale::Timescale::try_new(1000).expect("non-zero timescale"), core::time::Duration::from_secs(2)).unwrap();
/// let init = seg.init_segment().unwrap();      // ftyp + moov
/// assert_eq!(&init[4..8], b"ftyp");
/// # use broadcast_common::{Stage, Timestamp};
/// # use transmux::mux_stage::MuxInput;
/// let sample = |sync| MuxInput::Sample { track: transmux::TrackId::new(1), sample: au(sync) };
/// seg.feed(sample(true), Timestamp::ZERO).unwrap();      // keyframe
/// seg.feed(sample(false), Timestamp::ZERO).unwrap();
/// assert!(seg.poll().is_none());               // still buffering: no cut yet
/// seg.feed(MuxInput::Flush, Timestamp::ZERO).unwrap();   // trailing segment
/// assert!(seg.poll().is_some());
/// ```
pub struct Segmenter {
    tracks: Vec<TrackState>,
    movie_timescale: broadcast_core::timescale::Timescale,
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
    /// Media segments finished but not yet polled by the caller, in playlist
    /// order.
    ready: VecDeque<SegmentOut>,
    /// Explicit discontinuity: when `true` the *next* cut is marked discontinuous.
    /// Reset to `false` after each cut.
    pending_discontinuity: bool,
}

impl Segmenter {
    /// Create a segmenter for `tracks`, cutting segments roughly every
    /// `target_duration` on the anchor track's keyframes.
    ///
    /// The anchor is the first video track, falling back to the first
    /// [`is_anchor_capable`] track (audio-only input).
    /// `movie_timescale` matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration` is zero, or no track is anchor-capable (every track is
    /// section-carried, so no cut boundary could ever advance — see
    /// [`choose_anchor`]).
    pub fn new(
        tracks: Vec<TrackSpec>,
        movie_timescale: impl Into<broadcast_core::timescale::Timescale>,
        target_duration: core::time::Duration,
    ) -> Result<Self> {
        let movie_timescale: broadcast_core::timescale::Timescale = movie_timescale.into();
        if tracks.is_empty() {
            return Err(Error::InvalidInput("segmenter needs at least one track"));
        }
        if target_duration.is_zero() {
            return Err(Error::InvalidInput("target_duration must be positive"));
        }
        let target_duration_secs = target_duration.as_secs_f64();
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

        let anchor_timescale = tracks[anchor].timescale.get() as f64;
        let target_ticks = (target_duration_secs * anchor_timescale) as u64;
        let target_ticks = target_ticks.max(1); // never a zero-length target

        let tracks = tracks
            .into_iter()
            .map(|spec| TrackState {
                spec,
                pending: Vec::new(),
                base_decode: 0,
                flush_clock: MediaClock::new(),
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
        })
    }

    /// Start `track_id`'s first `tfdt` (`baseMediaDecodeTime`, ISO/IEC
    /// 14496-12:2026 §8.8.12) at `base` media-timescale ticks instead of 0, so
    /// a track that starts later than the others keeps that offset (issue
    /// #1021). Only meaningful before the first cut.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] when `track_id` is not one of the segmenter's
    /// tracks.
    pub fn set_start_decode_time(&mut self, track_id: TrackId, base: u64) -> Result<()> {
        let t = self
            .tracks
            .iter_mut()
            .find(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput(
                "set_start_decode_time: unknown track_id",
            ))?;
        t.base_decode = base;
        Ok(())
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`). Stable for the
    /// life of the segmenter; write it once before any media segment.
    pub fn init_segment(&self) -> Result<Vec<u8>> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        build_init_segment(&specs, self.movie_timescale)
    }

    /// Buffer one coded sample for `track`, in decode order.
    ///
    /// If this is a sync sample on the anchor track and the anchor has already
    /// buffered at least the target duration, the buffered samples are cut into a
    /// media segment (emitted by [`Stage::poll`]) *before* this sample is
    /// buffered — so the new sample opens the next segment on a random-access
    /// point.
    ///
    /// Anchor progress is measured by [`MediaClock`]: each anchor sample's own
    /// `duration` when that is a real, non-zero span, else the `dts` delta from
    /// the previous anchor sample. `duration` alone is not enough — a
    /// `Some(0)` duration is routine on live input (see [`MediaClock`]) and
    /// would otherwise freeze the accumulator so no segment is ever cut.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track` matches no track, the underlying
    /// [`build_media_segment`] fails while cutting, or this track already holds
    /// [`MAX_PENDING_SAMPLES_PER_TRACK`] un-cut samples (no anchor sync sample
    /// to cut on — send [`MuxInput::Flush`] to close a trailing partial
    /// segment).
    fn push_sample(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track)
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

    /// Finalize the trailing partial segment. A no-op if nothing is buffered.
    ///
    /// # Errors
    /// Propagates a [`build_media_segment`] failure.
    fn flush_segment(&mut self) -> Result<()> {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.cut_segment()?;
        }
        Ok(())
    }

    /// Cut the buffered samples across all tracks into one media segment, advance
    /// each track's `base_decode`, and clear the buffers.
    fn cut_segment(&mut self) -> Result<()> {
        let (seg, anchor_duration) = {
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
            let bytes = build_media_segment(self.next_seq, &frags)?;
            let mut anchor_clock = MediaClock::resumed_at(Some(
                i64::try_from(self.tracks[self.anchor].base_decode).unwrap_or(i64::MAX),
            ));
            let anchor_duration = self.tracks[self.anchor]
                .pending
                .iter()
                .map(|s| anchor_clock.tick(s))
                .sum();
            (bytes, anchor_duration)
        }; // immutable borrow of `self.tracks` ends here, before the mutation below

        // The discontinuity flag is the explicit `MuxInput::Discontinuity`
        // request only: `Segmenter` has no API that changes a track's
        // `TrackSpec` after construction, so the init segment can never change
        // between cuts (the old byte-compare rebuilt the whole `moov` on every
        // cut to detect a change that cannot happen, audit r05-O5).
        let discontinuous = self.pending_discontinuity;
        self.pending_discontinuity = false;

        let sequence = u64::from(self.next_seq.saturating_sub(1));
        // Per-track parts before the buffers are cleared: each track's own
        // start, duration, bytes and sample count in this segment.
        let tracks: Vec<crate::mux_stage::TrackPart> = self
            .tracks
            .iter()
            .filter(|t| !t.pending.is_empty())
            .map(|t| {
                let mut clock =
                    MediaClock::resumed_at(Some(i64::try_from(t.base_decode).unwrap_or(i64::MAX)));
                let duration: u64 = t.pending.iter().map(|s| clock.tick(s)).sum();
                crate::mux_stage::TrackPart {
                    track: t.spec.track_id,
                    start: t.base_decode,
                    duration,
                    timescale: t.spec.timescale.get(),
                    bytes: t.pending.iter().map(|s| s.data.len() as u64).sum(),
                    samples: u32::try_from(t.pending.len()).unwrap_or(u32::MAX),
                }
            })
            .collect();
        let track = self.tracks[self.anchor].spec.track_id;
        let timescale = self.tracks[self.anchor].spec.timescale.get();
        let start = self.tracks[self.anchor].base_decode;
        let independent = self.tracks[self.anchor]
            .pending
            .first()
            .is_some_and(|s| s.flags.is_sync);

        self.next_seq += 1;
        for t in &mut self.tracks {
            // Same duration-then-dts-delta rule as the anchor accumulator (see
            // `MediaClock`): a plain `duration` sum would leave `base_decode`
            // at 0 forever on a `duration: Some(0)` stream, collapsing every
            // segment's `tfdt` onto the same decode time.
            let clock = &mut t.flush_clock;
            let dur: u64 = t.pending.iter().map(|s| clock.tick(s)).sum();
            t.base_decode += dur;
            t.pending.clear();
        }
        self.anchor_pending_dur = 0;
        self.ready.push_back(SegmentOut {
            summary: SegmentSummary {
                track,
                sequence,
                start,
                duration: anchor_duration,
                timescale,
                bytes: seg.len() as u64,
                independent,
                discontinuous,
            },
            bytes: bytes::Bytes::from(seg),
            tracks,
        });
        Ok(())
    }
}

/// [`Stage`] adoption: `In = MuxInput`, `Out = MuxOutput`. A [`Segmenter`]'s
/// track set is fixed at construction, so a [`MuxInput::Track`] is refused
/// naming the reason; [`MuxInput::Discontinuity`] marks the next cut (RFC 8216
/// §4.3.2.3) and [`MuxInput::Flush`] closes the trailing partial segment.
impl Stage for Segmenter {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Sample { track, sample } => self.push_sample(track, sample),
            MuxInput::Discontinuity { .. } => {
                self.pending_discontinuity = true;
                Ok(())
            }
            MuxInput::Flush => self.flush_segment(),
            MuxInput::Track(_) => Err(Error::InvalidInput(
                "a Segmenter's track set is fixed at construction; start a new Segmenter \
                 for a changed track set",
            )),
            MuxInput::TrackRemoved { .. } => Err(Error::UnsupportedFeature(
                "cmaf: a track removed mid-stream changes the init segment",
            )),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready
            .pop_front()
            .map(|segment| MuxOutput::Segment(Box::new(segment)))
    }

    fn finish(&mut self) -> Result<()> {
        self.flush_segment()
    }

    /// `saturated` once any track holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples — the point past which [`feed`](Stage::feed) errors
    /// rather than buffering further, so a cooperative driver must stop
    /// feeding (and [`finish`](Stage::finish) to close the trailing partial
    /// segment) instead of growing this segmenter without bound waiting for a
    /// sync sample that may never arrive. Below the bound there is no hard cap
    /// to report, so `want_bytes` stays the honest "no preference" default.
    fn demand(&self) -> Demand {
        Demand::saturated_if(
            self.tracks
                .iter()
                .any(|t| t.pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK),
        )
    }
}

#[cfg(test)]
mod last_dts_tests {
    use super::*;

    fn dts_only(dts: i64) -> Sample {
        Sample::new(Vec::new(), Some(dts), None, None, false).expect("valid timing")
    }

    #[test]
    fn tick_records_the_dts_for_the_next_window() {
        let mut clock = MediaClock::new();
        assert_eq!(clock.tick(&dts_only(100)), 0);
        assert_eq!(clock.last_dts(), Some(100));
    }

    #[test]
    fn resumed_clock_measures_from_the_previous_sample() {
        let mut clock = MediaClock::resumed_at(Some(100));
        assert_eq!(clock.tick(&dts_only(140)), 40);
    }
}
