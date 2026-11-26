//! Low-latency DASH — chunked CMAF packaging + LL-DASH MPD signalling.
//!
//! Whole-segment CMAF ([`crate::segmenter::Segmenter`], [`crate::media::CmafMuxStage`])
//! delivers one `moof`+`mdat` per segment: a client cannot start playing a
//! segment until the whole thing has been produced, so end-to-end latency is at
//! least one segment duration. **Low-latency DASH** (DASH-IF Low-latency Live
//! Interoperability, "LL-DASH IOP") cuts that by splitting each CMAF segment
//! into several **CMAF chunks** — each a self-contained `moof`+`mdat` covering a
//! sub-run of the segment's samples — delivered (over HTTP chunked transfer) as
//! they are produced. The client can begin decoding the first chunk while the
//! rest of the segment is still being generated.
//!
//! # Chunked-CMAF structure (ISO/IEC 14496-12:2026)
//!
//! A CMAF chunk is exactly the fragment structure the batch builder
//! [`crate::build_media_segment`] already emits, but scoped to K samples:
//!
//! - the **first chunk** of a segment carries the segment-type box
//!   `styp` (§8.14.2) followed by `moof` (§8.8.4) + `mdat` (§8.1.1);
//! - **subsequent chunks** are a bare `moof` + `mdat`.
//!
//! Each chunk's `moof` has its own `mfhd.sequence_number` (§8.8.5) — contiguous
//! and increasing across all chunks of all segments — and each track fragment's
//! `tfdt.baseMediaDecodeTime` (§8.8.12) is the decode time of that chunk's first
//! sample. Concatenating a segment's chunks yields the same coded samples, in
//! the same decode order, with the same per-track decode timeline, as the
//! whole-segment [`crate::build_media_segment`] output — chunking splits, it
//! never loses, duplicates, or reorders a sample. Segment boundaries stay
//! keyframe-aligned (the anchor track's first chunk of every segment starts on a
//! sync sample), exactly as [`crate::segmenter::Segmenter`] guarantees.
//!
//! # LL-DASH MPD signalling (ISO/IEC 23009-1 + DASH-IF LL IOP)
//!
//! [`LlDashPackager`] renders an MPD that advertises the chunked availability so
//! a player can request a segment before it is complete:
//!
//! - **`SegmentTemplate@availabilityTimeComplete="false"`** (ISO/IEC 23009-1
//!   §5.3.9.5.3) — a segment becomes available (its first chunk) before the whole
//!   segment is produced.
//! - **`SegmentTemplate@availabilityTimeOffset`** (§5.3.9.5.3) — how many seconds
//!   *before* the nominal segment-complete time the segment first becomes
//!   available. Per DASH-IF LL IOP this is `segment_duration − chunk_duration`
//!   (the first chunk is ready one chunk-duration into the segment).
//! - **`MPD/ServiceDescription/Latency@target`** (§5.13.2) — the target
//!   end-to-end latency in milliseconds, with an optional `<PlaybackRate>` giving
//!   the min/max catch-up rate.
//! - **`MPD/UTCTiming`** (§5.8.4.11) — how a client obtains wall-clock UTC. A
//!   `dynamic` MPD is timed against `availabilityStartTime`, which is only usable
//!   if the client's clock agrees with the server's; DASH-IF LL IOP §4.1 makes
//!   the element effectively required for a low-latency presentation. The
//!   `@schemeIdUri` (one of the six ISO-registered `urn:mpeg:dash:utc:*:2014`
//!   values) and `@value` are supplied by the caller.
//! - `MPD@type="dynamic"` with `@availabilityStartTime` (§5.3.1.2) — LL-DASH is a
//!   live profile.
//!
//! `<ProducerReferenceTime>` (§5.12) is **optional** and is *not* emitted by this
//! packager: it requires a wall-clock capture timestamp per segment that the
//! samples-in IR does not carry. A caller with that timing can add it out of
//! band; its absence does not affect the availability signalling above.

use crate::TrackId;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::{Demand, Stage, Timestamp};

use crate::error::{Error, Result};
use crate::mux_stage::{MuxInput, MuxOutput};
use crate::pipeline::{FragmentTrackData, Sample, TrackSpec, build_chunk, build_init_segment};
use crate::segmenter::{MAX_PENDING_SAMPLES_PER_TRACK, MediaClock, choose_anchor};

// ===========================================================================
// LlSegmenter — chunked-CMAF output packager
// ===========================================================================

/// Per-track accumulation state for the segment currently being built.
struct TrackState {
    spec: TrackSpec,
    /// Samples buffered for the current (not-yet-cut) segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of the first *pending* sample (media-timescale ticks) — the
    /// `tfdt.baseMediaDecodeTime` of this track's next chunk.
    base_decode: u64,
    /// Advances `base_decode` past each sample as it is drained into a chunk,
    /// under the same duration-then-dts-delta rule as the anchor accumulator
    /// (see [`MediaClock`]). A separate clock instance from the anchor's: this
    /// one walks *this* track's samples once at drain time. A plain `duration`
    /// sum would pin `base_decode` at 0 forever on a `duration: Some(0)`
    /// stream, collapsing every chunk's `tfdt` onto the same decode time.
    flush_clock: MediaClock,
}

/// A stateful **chunked** CMAF segmenter for low-latency DASH.
///
/// Same segmentation state machine as [`crate::segmenter::Segmenter`] — segments
/// are cut on the anchor track's sync samples once the target duration is reached
/// — but each segment is emitted as an ordered list of chunks: one
/// `moof`+`mdat` per `chunk_samples` anchor-track samples (e.g. `1` for per-frame
/// LL). Concatenating a segment's chunks reproduces the whole-segment
/// [`crate::build_media_segment`] coded-sample stream (see the module docs).
///
/// ```
/// use broadcast_common::Stage;
/// use transmux::mux_stage::MuxOutput;
/// use transmux::{CodecConfig, LlSegmenter, Sample, TrackSpec};
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
/// // 2 s target segments, one video frame per chunk (per-frame LL).
/// let mut seg = LlSegmenter::new(vec![spec()], broadcast_core::timescale::Timescale::try_new(1000).expect("non-zero timescale"), core::time::Duration::from_secs(2), 1).unwrap();
/// let init = seg.init_segment().unwrap();       // ftyp + moov
/// assert_eq!(&init[4..8], b"ftyp");
/// seg.push(transmux::TrackId::new(1), au(true)).unwrap();               // keyframe
/// seg.push(transmux::TrackId::new(1), au(false)).unwrap();
/// let mut chunks = 0;
/// while seg.poll().is_some() {                  // write each chunk over HTTP
///     chunks += 1;
/// }
/// assert_eq!(chunks, 2);                        // one chunk per pushed frame
/// seg.flush().unwrap();                         // trailing chunks (none pending here)
/// ```
pub struct LlSegmenter {
    tracks: Vec<TrackState>,
    movie_timescale: broadcast_core::timescale::Timescale,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Number of anchor-track samples per chunk (>= 1).
    chunk_samples: usize,
    /// Buffered duration of the anchor's `pending` samples (media-timescale ticks).
    anchor_pending_dur: u64,
    /// Anchor-progress clock: advances `anchor_pending_dur` from each anchor
    /// sample's `duration`, or from its `dts` delta when `duration` is absent
    /// or zero (see [`MediaClock`]).
    anchor_clock: MediaClock,
    /// `mfhd.sequence_number` of the next chunk, 1-based and contiguous.
    next_seq: u32,
    /// 1-based number of the segment currently being built.
    current_segment: u64,
    /// `true` until the first chunk of the current segment has been emitted.
    segment_open: bool,
    /// Chunks finished but not yet polled, as [`MuxOutput::Bytes`] — a chunk is
    /// a fragment, not a whole segment, so it carries no
    /// [`SegmentSummary`](crate::mux_stage::SegmentSummary).
    ready: VecDeque<MuxOutput>,
}

impl LlSegmenter {
    /// Create a chunked segmenter for `tracks`, cutting segments roughly every
    /// `target_duration` on the anchor track's keyframes, and subdividing
    /// each segment into chunks of `chunk_samples` anchor-track samples.
    ///
    /// The anchor is chosen by the shared
    /// [`crate::segmenter::choose_anchor`] — the first **video**
    /// track (any codec, not just AVC), falling back to the first
    /// anchor-capable track — exactly as [`crate::segmenter::Segmenter`].
    /// `movie_timescale` matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration` is zero, `chunk_samples == 0`,
    /// or no track is anchor-capable (every track is section-carried).
    pub fn new(
        tracks: Vec<TrackSpec>,
        movie_timescale: impl Into<broadcast_core::timescale::Timescale>,
        target_duration: core::time::Duration,
        chunk_samples: usize,
    ) -> Result<Self> {
        let movie_timescale = movie_timescale.into();
        if tracks.is_empty() {
            return Err(Error::InvalidInput("ll segmenter needs at least one track"));
        }
        if target_duration.is_zero() {
            return Err(Error::InvalidInput("target_duration must be positive"));
        }
        let target_duration_secs = target_duration.as_secs_f64();
        if chunk_samples == 0 {
            return Err(Error::InvalidInput("chunk_samples must be >= 1"));
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
        // `init_segment`/a chunk build actually needs the `trak` (this
        // constructor does not itself need to build one). The caller must
        // pre-filter first (e.g. with
        // `tracks.retain(|t| t.config.is_muxable_in_bmff())`) if it wants
        // to drop such tracks rather than fail.

        // Anchor = first **video** track (any `CodecConfig::is_video` codec),
        // else the first anchor-capable track — the shared rule
        // `Segmenter`/`ts_hls` use. This used to match only
        // `CodecConfig::Avc`, so an HEVC+AAC media with audio first anchored on
        // the *audio* track and its segments did not begin on an IRAP.
        let anchor = choose_anchor(tracks.iter().map(|t| &t.config))?;

        let anchor_timescale = tracks[anchor].timescale.get() as f64;
        let target_ticks = ((target_duration_secs * anchor_timescale) as u64).max(1);

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
            chunk_samples,
            anchor_pending_dur: 0,
            anchor_clock: MediaClock::new(),
            next_seq: 1,
            current_segment: 1,
            segment_open: false,
            ready: VecDeque::new(),
        })
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`). Stable for
    /// the life of the segmenter; write it once before any chunk.
    pub fn init_segment(&self) -> Result<Vec<u8>> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        build_init_segment(&specs, self.movie_timescale)
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// When the anchor track reaches a sync sample past the target duration, the
    /// current segment is finalized (all its remaining buffered samples flushed
    /// as a final chunk) *before* this sample is buffered, so the new keyframe
    /// opens the next segment's first chunk on a random-access point.
    ///
    /// Anchor progress comes from [`MediaClock`]: each anchor sample's own
    /// `duration` when that is a real, non-zero span, else the `dts` delta from
    /// the previous anchor sample — `duration` alone stalls on the `Some(0)`
    /// durations live input legitimately produces.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, a chunk build
    /// fails, or this track already holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples (no anchor sync sample to cut on — call
    /// [`flush`](Self::flush) to close a trailing partial segment).
    pub fn push(&mut self, track_id: TrackId, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        // Segment boundary: anchor keyframe past target → flush the segment.
        if idx == self.anchor
            && sample.flags.is_sync
            && self.anchor_pending_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty()
        {
            self.finish_segment()?;
        }

        if self.tracks[idx].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
            return Err(crate::segmenter::no_sync_sample_error());
        }

        if idx == self.anchor {
            self.anchor_pending_dur += self.anchor_clock.tick(&sample);
        }
        self.tracks[idx].pending.push(sample);

        // Emit a chunk once the anchor has buffered a full chunk's worth of
        // samples (but hold the segment's last chunk until the boundary/flush so
        // audio that trails the last video frame stays in the same segment).
        if idx == self.anchor
            && self.tracks[self.anchor].pending.len() >= self.chunk_samples
            && self.anchor_pending_dur < self.target_ticks
        {
            self.emit_chunk(false)?;
        }
        Ok(())
    }

    /// Finalize the trailing segment at end-of-stream. A no-op if nothing is
    /// buffered. Any remaining chunks are appended to the ready queue.
    ///
    /// # Errors
    /// Propagates a chunk-build failure.
    pub fn flush(&mut self) -> Result<()> {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.finish_segment()?;
        }
        Ok(())
    }

    /// Flush the whole current segment: emit all remaining buffered anchor
    /// samples (and every non-anchor sample) as chunks, then open the next
    /// segment. The final chunk of a segment carries any trailing samples.
    fn finish_segment(&mut self) -> Result<()> {
        while self.tracks[self.anchor].pending.len() > self.chunk_samples {
            self.emit_chunk(false)?;
        }
        // Final chunk of the segment (anchor remainder + all other tracks).
        self.emit_chunk(true)?;

        self.current_segment += 1;
        self.segment_open = false;
        self.anchor_pending_dur = 0;
        Ok(())
    }

    /// Emit one chunk. If `final_chunk`, drain every buffered sample of every
    /// track; otherwise drain `chunk_samples` anchor samples and no other track's
    /// samples (non-anchor samples ride the segment's final chunk, matching the
    /// whole-segment layout's single per-track run).
    fn emit_chunk(&mut self, final_chunk: bool) -> Result<()> {
        let is_start = !self.segment_open;
        let anchor = self.anchor;

        // The decode span this chunk covers, in the anchor's timescale: the
        // first `chunk_samples` of the anchor's pending samples. Every track then
        // drains its own samples that fall inside that span, so audio rides each
        // chunk instead of only the segment's final one (audit r05-W27).
        let anchor_scale = self.tracks[anchor].spec.timescale.get() as u64;
        let anchor_take = if final_chunk {
            self.tracks[anchor].pending.len()
        } else {
            self.chunk_samples.min(self.tracks[anchor].pending.len())
        };
        let chunk_span = if final_chunk {
            u64::MAX
        } else {
            let t = &self.tracks[anchor];
            let mut clock =
                MediaClock::resumed_at(Some(i64::try_from(t.base_decode).unwrap_or(i64::MAX)));
            // Seed with the anchor's own first pending sample's real dts so the
            // delta fallback measures from the right baseline.
            let mut span: u64 = 0;
            for s in &t.pending[..anchor_take] {
                span += clock.tick(s);
            }
            span
        };

        // Decide, per track, how many leading pending samples this chunk drains.
        let take_counts: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if final_chunk {
                    return t.pending.len();
                }
                if i == anchor {
                    return anchor_take;
                }
                let track_scale = t.spec.timescale.get() as u64;
                let track_span = chunk_span.saturating_mul(track_scale) / anchor_scale;
                take_span(t, track_span)
            })
            .collect();

        // Nothing to emit (e.g. flush with only-empty tracks) → skip.
        if take_counts.iter().all(|&n| n == 0) {
            return Ok(());
        }

        let seq = self.next_seq;
        let chunk_bytes = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .zip(&take_counts)
                .filter(|&(_, &n)| n > 0)
                .map(|(t, &n)| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.base_decode,
                    samples: &t.pending[..n],
                })
                .collect();
            build_chunk(seq, &frags, is_start, &[])?
        };

        // Advance decode times and drop the drained samples. The drained
        // ranges are contiguous and in decode order across calls, so one
        // per-track `flush_clock` walks each sample exactly once and its
        // dts-delta fallback stays correct across chunk boundaries.
        for (t, &n) in self.tracks.iter_mut().zip(&take_counts) {
            let clock = &mut t.flush_clock;
            let dur: u64 = t.pending[..n].iter().map(|s| clock.tick(s)).sum();
            t.base_decode += dur;
            t.pending.drain(..n);
        }

        self.next_seq += 1;
        self.segment_open = true;
        self.ready
            .push_back(MuxOutput::Bytes(bytes::Bytes::from(chunk_bytes)));
        Ok(())
    }
}

/// How many of `t`'s pending samples fit inside a decode span of `target_ticks`
/// (in this track's timescale).
///
/// A lower-rate track (audio) is drained by *time*, not by the anchor's sample
/// count: a chunk that takes 3 video AUs also takes every audio frame inside
/// those 3 AUs, so audio accompanies the video in each chunk (audit r05-W27)
/// instead of waiting for the segment's final chunk. Always takes at least one
/// sample when one is available, so an over-target first sample cannot stall.
fn take_span(t: &TrackState, target_ticks: u64) -> usize {
    if t.pending.is_empty() {
        return 0;
    }
    let mut clock = MediaClock::resumed_at(Some(i64::try_from(t.base_decode).unwrap_or(i64::MAX)));
    let mut span: u64 = 0;
    let mut end = 0usize;
    for s in &t.pending {
        let next = clock.tick(s);
        if end > 0 && span + next > target_ticks {
            break;
        }
        span += next;
        end += 1;
    }
    end.max(1).min(t.pending.len())
}

/// [`Stage`] adoption: `In = MuxInput`, `Out = MuxOutput` — the same muxer
/// vocabulary every other muxer stage speaks. Each finished chunk is one
/// [`MuxOutput::Bytes`]; the inherent [`push`](Self::push) /
/// [`flush`](Self::flush) keep working unchanged.
impl Stage for LlSegmenter {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Sample { track, sample } => self.push(track, sample),
            // A chunk carries its own `tfdt`, so a decode-time jump needs no
            // separate signalling in the LL-DASH chunk stream.
            MuxInput::Discontinuity { .. } => Ok(()),
            MuxInput::Flush => self.flush(),
            MuxInput::Track(_) => Err(Error::InvalidInput(
                "an LlSegmenter's track set is fixed at construction; start a new \
                 LlSegmenter for a changed track set",
            )),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()
    }

    /// `saturated` once any track holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples — the same bound (and reasoning) as
    /// [`Segmenter`](crate::segmenter::Segmenter)'s: past it
    /// [`feed`](Stage::feed) errors rather than buffering a stream that never
    /// produces the sync sample a segment must open on.
    fn demand(&self) -> Demand {
        Demand::saturated_if(
            self.tracks
                .iter()
                .any(|t| t.pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK),
        )
    }
}

// The LL-DASH MPD packager renders and rewrites XML (quick-xml), so it is
// `std`-only; the chunked-CMAF segmenter above stays `no_std` + `alloc`.
#[cfg(feature = "std")]
mod mpd;
#[cfg(feature = "std")]
pub use mpd::*;
