//! Classic HLS with MPEG-2 TS media segments — RFC 8216 + ISO/IEC 13818-1.
//!
//! Where [`HlsPackager`](crate::media::HlsPackager) emits **CMAF-HLS** (fMP4
//! `.m4s` segments described by an `#EXT-X-MAP`-bearing media playlist), this
//! module emits **classic HLS**: MPEG-2 Transport Stream `.ts` media segments
//! plus an RFC 8216 media playlist referencing them. Each `.ts` segment is a
//! self-contained MPEG-2 TS (its own PAT + PMT then a keyframe-aligned PES),
//! so it is independently decodable — there is no init/`#EXT-X-MAP` segment, as
//! that is a CMAF-only concept (RFC 8216 §4.3.2.5 applies only to fMP4 media).
//!
//! # Behaviour
//!
//! [`StreamingTsHlsSegmenter`] segments at keyframe boundaries on the **anchor
//! track** (the first video track, else the first track whose samples can
//! advance a clock — never a section-carried track, whose `duration` *and*
//! `dts` are always `None`; see [`crate::segmenter::choose_anchor`]), cutting a
//! new segment on the first sync sample at or past the target duration —
//! mirroring [`Segmenter`](crate::segmenter::Segmenter)'s CMAF rule so every
//! video segment begins on a random-access point. Every track's samples are
//! partitioned across the segments by decode time, so the concatenation of all
//! segments carries the full input (no sample dropped, duplicated, or
//! reordered). Each segment is then muxed via the shared [`crate::ts_mux`]
//! machinery, re-emitting PAT + PMT at its start (ISO/IEC 13818-1 §2.4.4 PSI
//! repetition — a receiver joining mid-stream must find the PSI at each
//! segment boundary).
//!
//! The segmenter is given every collected track and fed every
//! sample in decode order; the resulting playlist is a VOD media playlist: `#EXTM3U`,
//! `#EXT-X-VERSION`, `#EXT-X-TARGETDURATION` (≥ every `#EXTINF`),
//! `#EXT-X-MEDIA-SEQUENCE`, one `#EXTINF` + `.ts` URI per segment, and a
//! trailing `#EXT-X-ENDLIST` (RFC 8216 §4.3.3). `#EXT-X-DISCONTINUITY`
//! (RFC 8216 §4.3.2.3) and `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216
//! §4.3.3.3) are rendered from the segmenter's own discontinuity state.
//!
//! # Cut policy
//!
//! [`StreamingTsHlsSegmenter::with_cut_policy`] selects a
//! [`SegmentCutPolicy`]: the default `AnyRap` cuts on
//! any sync sample; `IdrOnly` cuts only on an IDR picture and drops leading
//! access units until the first one (Apple HLS Authoring Spec 7.4,
//! `docs/hls-apple-conformance/`).
//!
//! # Input — [`StreamingTsHlsSegmenter`]
//!
//! The segmenter speaks the muxer stage vocabulary
//! ([`crate::mux_stage::MuxInput`]/[`crate::mux_stage::MuxOutput`]): feed it a
//! track announcement or update, one coded sample at a time, a timeline
//! discontinuity or an explicit flush, and pull a finished `.ts`
//! [`MuxOutput::Segment`] — its bytes
//! plus a filled [`SegmentSummary`] — whenever the anchor track crosses a
//! keyframe past the target duration. [`Stage::finish`] flushes the trailing
//! partial segment; [`StreamingTsHlsSegmenter::playlist`] renders a rolling
//! media playlist over a configurable sliding window, advancing
//! `#EXT-X-MEDIA-SEQUENCE` and `#EXT-X-DISCONTINUITY-SEQUENCE` as older
//! segments roll off (RFC 8216 §6.2.1 defines a segment's Discontinuity
//! Sequence Number as the header value plus the `#EXT-X-DISCONTINUITY` tags
//! still visible ahead of it in the *current* playlist, so the header only
//! needs to advance once a discontinuous segment's own inline tag has rolled
//! out of the window), and omitting `#EXT-X-ENDLIST` until `finish` has been
//! called.
//!
//! Anchor selection, the cut decision and duration accounting all come from
//! [`crate::segmenter::choose_anchor`] and [`crate::segmenter::MediaClock`]
//! (shared with the other segmenters), plus this module's private
//! `is_cut_point`/`segment_duration`/`placement_secs` helpers.
//!
//! # Spec
//!
//! - **HLS media playlist**: RFC 8216 §4.1 / §4.3.2 (`#EXTINF`) / §4.3.3
//!   (`#EXT-X-TARGETDURATION`, `#EXT-X-MEDIA-SEQUENCE`, `#EXT-X-ENDLIST`).
//! - **TS segment constraints (PAT/PMT at each segment start)**:
//!   ISO/IEC 13818-1 §2.4.4 — PSI is periodically repeated so any access point
//!   is self-describing.

use crate::TrackId;
use crate::units::ROUNDING_DIVISOR;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use broadcast_common::timescale::Timescale;
use broadcast_common::{Demand, Stage, Timestamp};
use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};

use crate::error::{Error, Result};
use crate::media::Track;
use crate::mux_stage::{MuxInput, MuxOutput, SegmentOut, SegmentSummary, TrackPart};
use crate::nal::{NalCodec, access_unit_is_idr};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::segment_cut::SegmentCutPolicy;
use crate::segmenter::{
    MAX_PENDING_SAMPLES_PER_TRACK, MediaClock, choose_anchor, no_sync_sample_error,
};
use crate::ts_mux::{TsContinuity, mux_tracks_at_continuing};

/// Outcome of [`StreamingTsHlsSegmenter::update_track`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateTrackOutcome {
    /// A discontinuity-triggering field differed; a discontinuity was armed.
    Changed,
    /// No discontinuity-triggering field differed; the stored spec was still
    /// refreshed.
    Unchanged,
    /// No track with that `track_id` exists; nothing was modified.
    Unknown,
}

/// Default `#EXT-X-VERSION` for a classic (TS-segment) media playlist. Version 3
/// is the floor for floating-point `#EXTINF` durations (RFC 8216 §7).
const DEFAULT_HLS_VERSION: u8 = 3;

// ── Shared segment-cutting arithmetic ──────────────────────────────────────
//
// The segmenter cuts segments on the anchor track's keyframe rule and derives
// every duration below from the one [`MediaClock`], so the placement and
// duration rules cannot drift apart (issue #571).

// Anchor selection (`is_anchor_capable`/`choose_anchor`) and the anchor
// progress clock (`MediaClock`) are the *shared* implementations in
// [`crate::segmenter`] — this module used to carry its own copies, so the four
// segmenters could drift apart on which track may be the anchor. See
// [`choose_anchor`](crate::segmenter::choose_anchor) for the rule and why a
// section-only track set is a construction error rather than a silent stall.

/// Decode-start time of one sample of a **non-anchor** track, in seconds on
/// the segmentation timeline (`0.0` = the anchor's first sample), used to
/// decide which segment that sample belongs to.
///
/// Normally the running per-track accumulator `acc_ticks` (advanced by
/// [`MediaClock`]). A sample carrying **no `duration` of its own but an
/// absolute `dts`** is instead placed by that `dts`, relative to
/// `origin_secs` (the anchor's first sample's decode time in seconds): that is
/// exactly a section-carried sample (ISO/IEC 13818-1 §2.4.4 — an SCTE-35
/// `splice_info_section`, say), for which the accumulator can never advance,
/// so without this rule *every* section sample would be placed at time `0.0`
/// and land in segment 0 no matter how late it actually occurs.
///
/// [`StreamingTsHlsSegmenter::cut_segment`] calls this for every non-anchor
/// track, so a sample of a track with a different timescale lands in the same
/// segment an anchor-relative split would place it.
///
/// A section sample carrying *neither* a duration nor a dts has no time
/// information anywhere in the IR; it keeps the accumulator's value, i.e. it
/// stays with the samples around it.
fn placement_secs(sample: &Sample, acc_ticks: u64, timescale: Timescale, origin_secs: f64) -> f64 {
    let timescale = f64::from(timescale.get());
    match (
        sample.timing.and_then(|t| t.duration()).filter(|&d| d != 0),
        sample.timing.and_then(|t| t.dts()),
    ) {
        (None, Some(dts)) => (dts.max(0) as f64 / timescale) - origin_secs,
        _ => acc_ticks as f64 / timescale,
    }
}

/// The segmentation timeline's origin in seconds: the anchor track's first
/// sample's absolute decode time, or `0.0` when the anchor carries no `dts`
/// (segment start times are then relative to the anchor's first sample, which
/// is the same thing).
fn timeline_origin_secs(anchor_first_dts: Option<i64>, anchor_scale: Timescale) -> f64 {
    anchor_first_dts.map_or(0.0, |d| d.max(0) as f64 / f64::from(anchor_scale.get()))
}

/// Convert a whole-seconds target duration into anchor-track timescale ticks
/// (never zero, so a segment can always close).
fn target_ticks_for(target_secs: u32, ts_scale: u32) -> u64 {
    (target_secs.max(1) as u64) * (ts_scale.max(1) as u64)
}

/// Segment duration, plus the RFC 8216 `#EXT-X-TARGETDURATION` ceiling
/// (whole seconds, rounded up) for `anchor_ticks` of buffered anchor media at
/// `ts_scale` ticks/second.
fn segment_duration(anchor_ticks: u64, ts_scale: Timescale) -> (Duration, u32) {
    let denom = u128::from(ts_scale.get());
    let nanos = (u128::from(anchor_ticks) * u128::from(broadcast_common::units::NANOS_PER_SEC)
        + denom / u128::from(ROUNDING_DIVISOR))
        / denom;
    let duration = Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX));
    let ceil_secs = anchor_ticks.div_ceil(ts_scale.as_u64()) as u32;
    (duration, ceil_secs)
}

/// True when the anchor should be cut *before* buffering the incoming sample:
/// the current segment already has content (`has_pending`), the sample is a
/// sync sample, and the buffered duration has reached the target — the
/// segmenter's incremental cut decision.
fn is_cut_point(has_pending: bool, is_sync: bool, buffered_ticks: u64, target_ticks: u64) -> bool {
    has_pending && is_sync && buffered_ticks >= target_ticks
}

// ── Streaming (incremental) TS-HLS segmentation for live input (issue #571) ─

/// Playlist-relevant metadata for one segment retained in the rolling window
/// — the segment bytes themselves are handed to the caller by
/// [`StreamingTsHlsSegmenter::push`]/[`finish`](StreamingTsHlsSegmenter::finish)
/// and are not kept here.
#[derive(Debug, Clone)]
struct WindowEntry {
    uri: String,
    duration: DecimalSeconds,
    discontinuous: bool,
}

/// Per-track accumulation state for [`StreamingTsHlsSegmenter`].
/// A cut the anchor triggered whose non-anchor audio has not yet caught up.
///
/// A live multiplex delivers a track's PES later than the video it
/// accompanies, by a jitter that varies per segment. Cutting the instant the
/// anchor reaches its next sync sample therefore closes each segment with
/// however much audio had arrived (zenith captures: 1.15 s to 1.34 s of audio
/// against 1.28 s of video), and Apple's `mediastreamvalidator` rejects the
/// playlist-vs-segment duration mismatch. The cut is held until every audio
/// track holds a sample at or past the boundary, bounded by
/// [`CATCH_UP_MAX_SECS`] of further anchor media.
#[derive(Debug, Clone, Copy)]
struct DeferredCut {
    /// Anchor samples (from the front of `pending`) that close the segment.
    anchor_split: usize,
    /// Anchor ticks those samples span: the segment's `EXTINF`.
    closed_anchor_dur: u64,
    /// The boundary: where the next segment starts, in seconds from the
    /// anchor's first sample.
    next_start_secs: f64,
}

/// Longest the streaming segmenter holds a cut for lagging audio, in seconds
/// of further anchor media. Beyond it the segment is cut with whatever audio
/// arrived, so a silent or stalled audio track cannot stop segments.
const CATCH_UP_MAX_SECS: u64 = 1;

struct StreamTrackState {
    spec: TrackSpec,
    /// Samples buffered so far that have not yet been flushed into a
    /// segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of `pending[0]`, in this track's media timescale ticks —
    /// the elapsed media time of every sample already flushed into an
    /// earlier segment. Mirrors `TrackState::base_decode` in
    /// [`Segmenter`](crate::segmenter::Segmenter).
    base_decode: u64,
    /// Advances `base_decode` past each sample as it is flushed, under the
    /// same duration-then-dts-delta rule as the anchor accumulator (see
    /// [`MediaClock`]). A plain `duration` sum would pin `base_decode` at 0
    /// forever on a `duration: Some(0)` stream, so every segment would be
    /// muxed at the same base DTS.
    flush_clock: MediaClock,
}

/// A stateful classic-HLS segmenter for live and whole-input segments,
/// mirroring how [`Segmenter`](crate::segmenter::Segmenter) drives incremental
/// CMAF fragment production.
///
/// Build it from the same [`TrackSpec`]s (in the same order) as the source
/// collected tracks, `push` coded samples in decode order, and pull finished
/// `.ts` segments from `push`'s return value; `finish` emits the final
/// partial segment. [`Self::playlist`] renders the current rolling media
/// playlist over the configured window.
///
/// ```
/// use transmux::{CodecConfig, Sample, TrackSpec};
/// use transmux::ts_hls::StreamingTsHlsSegmenter;
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
/// #     TrackSpec::new(transmux::TrackId::new(1), broadcast_common::timescale::Timescale::try_new(90_000).expect("non-zero timescale"), CodecConfig::Avc {
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
/// // 2 s target segments, keep the last 3 in the rolling playlist.
/// use broadcast_common::stage::{Stage, Timestamp};
/// use transmux::mux_stage::{MuxInput, MuxOutput};
/// let mut seg = StreamingTsHlsSegmenter::new(vec![spec()], 2, 3).unwrap();
/// Stage::feed(
///     &mut seg,
///     MuxInput::Sample { track: transmux::TrackId::new(1), sample: au(true) },
///     Timestamp::ZERO,
/// )
/// .unwrap();
/// Stage::finish(&mut seg).unwrap();
/// let mut segments = 0;
/// while let Some(output) = Stage::poll(&mut seg) {
///     if let MuxOutput::Segment(segment) = output {
///         assert_eq!(segment.bytes[0], 0x47); // MPEG-TS sync byte
///         segments += 1;
///     }
/// }
/// assert_eq!(segments, 1);
/// let playlist = seg.playlist().unwrap(); // rolling window, #EXT-X-ENDLIST after finish
/// assert!(playlist.contains("#EXT-X-ENDLIST"));
/// ```
pub struct StreamingTsHlsSegmenter {
    tracks: Vec<StreamTrackState>,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in whole seconds, as given to [`Self::new`] —
    /// retained (alongside the derived `target_ticks`) so
    /// [`Self::add_track`] can recompute `target_ticks` in the anchor-track's
    /// timescale if a late-added track becomes the new anchor (issue #624).
    target_secs: u32,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Buffered duration of the anchor's `pending` samples (media-timescale ticks).
    anchor_pending_dur: u64,
    /// A segment boundary the anchor reached while an audio track had not yet
    /// delivered the media up to it; see [`DeferredCut`].
    deferred_cut: Option<DeferredCut>,
    /// Anchor-progress clock: advances `anchor_pending_dur` from each anchor
    /// sample's `duration`, or from its `dts` delta when `duration` is absent
    /// or zero (see [`MediaClock`]).
    anchor_clock: MediaClock,
    /// The absolute decode time (seconds) of the anchor's **first** sample —
    /// the origin the time-based non-anchor split measures against, so a
    /// section sample's absolute `dts` can be compared with the
    /// relative-from-zero segment start times (see [`placement_secs`]).
    /// `None` until the first anchor sample is pushed.
    origin_secs: Option<f64>,
    /// Explicit discontinuity: when `true` the *next* cut is marked
    /// discontinuous. Reset to `false` after each cut.
    pending_discontinuity: bool,
    /// Which anchor pictures may open a segment ([`SegmentCutPolicy`]).
    cut_policy: SegmentCutPolicy,
    /// [`SegmentCutPolicy::IdrOnly`] only: no IDR has been seen since the
    /// session start or the last [`StreamingTsHlsSegmenter::mark_discontinuity`],
    /// so every sample is dropped until the anchor delivers one.
    awaiting_idr: bool,
    /// [`SegmentCutPolicy::IdrOnly`] only: the segment opened by the IDR that
    /// ends the current wait must be flagged discontinuous.
    discontinuity_at_resync: bool,
    /// Anchor access units dropped while awaiting an IDR.
    dropped_leading_aus: u64,
    /// Bound on the wait for the first IDR; `None` waits forever (strict).
    idr_wait: Option<crate::segment_cut::IdrWait>,
    /// [`SegmentCutPolicy::IdrOnly`] gave up waiting for an IDR and cuts on
    /// any sync sample for the rest of the stream.
    idr_fallback_active: bool,
    /// Set by [`Stage::finish`]; makes [`Self::playlist`] emit
    /// `#EXT-X-ENDLIST`.
    finished: bool,
    /// `#EXT-X-VERSION` written to the playlist.
    pub version: u8,
    /// URI prefix for generated `.ts` segment entries (`"{prefix}{sequence}.ts"`).
    pub uri_prefix: String,
    /// Maximum number of segments retained in the rolling playlist window.
    window: usize,
    /// The segments currently in the rolling window, oldest first.
    window_segments: VecDeque<WindowEntry>,
    /// Cross-segment continuity-counter state (§2.4.3.3): the segments of one
    /// stream concatenate into a single transport stream, so each PID's
    /// `continuity_counter` must keep counting across cuts (audit r05-W21).
    continuity: TsContinuity,
    /// Total number of segments ever cut (also the sequence number of the
    /// next segment).
    total_segments: u64,
    /// `#EXT-X-DISCONTINUITY-SEQUENCE`: count of discontinuous segments that
    /// have already rolled out of the window. RFC 8216 §6.2.1 defines a
    /// segment's Discontinuity Sequence Number as this header value *plus*
    /// the number of `#EXT-X-DISCONTINUITY` tags still visible in the
    /// current playlist preceding that segment's URI line — so a
    /// discontinuous segment's own inline tag (rendered while it's still in
    /// the window) already accounts for its own boundary; the header must
    /// only advance once that segment (and its tag) has rolled off, or every
    /// segment still in the window would be double-counted.
    discontinuity_sequence: u64,
    /// Running max `#EXT-X-TARGETDURATION` ceiling ever produced (monotonic,
    /// per RFC 8216 §4.3.3.1 — never shrinks even as segments roll off).
    target_duration: u32,
    /// Segments cut but not yet handed to the caller — the **single** source
    /// of truth for retrieval: [`Stage::poll`] drains it. Every cut lands here
    /// and *only* here (issue R2), so there is no value a caller can silently
    /// drop.
    ready: VecDeque<MuxOutput>,
}

fn audio_discontinuity_trigger(old: &CodecConfig, new: &CodecConfig) -> bool {
    if core::mem::discriminant(old) != core::mem::discriminant(new) {
        return false;
    }
    if old.audio_sample_rate() != new.audio_sample_rate()
        || old.audio_channel_field() != new.audio_channel_field()
    {
        return true;
    }
    match (old, new) {
        (CodecConfig::Aac { esds: e1, .. }, CodecConfig::Aac { esds: e2, .. }) => {
            let asc = |e: &isobmff::mp4esds::EsdsBox| {
                e.es_descriptor
                    .decoder_config
                    .as_ref()
                    .and_then(|dc| dc.decoder_specific_info.as_ref())
                    .map(|d| d.data.clone())
            };
            asc(e1) != asc(e2)
        }
        _ => false,
    }
}

impl StreamingTsHlsSegmenter {
    /// Start every track's decode clock at `start` instead of zero, so a run
    /// that continues an earlier one (a source reconnect) carries on from
    /// where it ended. Call before the first sample; a no-op
    /// once any track holds samples.
    pub fn set_start_time(&mut self, start: Duration) {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            return;
        }
        for t in &mut self.tracks {
            let ticks = start
                .as_nanos()
                .saturating_mul(u128::from(t.spec.timescale.get()))
                / u128::from(broadcast_common::units::NANOS_PER_SEC);
            t.base_decode = u64::try_from(ticks).unwrap_or(u64::MAX);
        }
    }

    /// Create a streaming segmenter for `tracks`, cutting segments roughly
    /// every `target_secs` seconds (clamped to at least 1) on the anchor
    /// track's keyframes, and keeping at most `window` segments in the
    /// rolling media playlist returned by [`Self::playlist`].
    ///
    /// The anchor is the first **video** track (any codec), else the first
    /// anchor-capable track — [`crate::segmenter::choose_anchor`],
    /// the same rule the other three segmenters use.
    /// `tracks` must be given in the
    /// same order the caller will later `push` matching `track_id`s, and in
    /// the same order as the source track set, so the muxed PID/PMT
    /// layout is the one a whole-input drive produces.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate
    /// `track_id`s, `window` is `0`, or no track is anchor-capable (e.g.
    /// every track is a section-carried `CodecConfig::Data` track, whose
    /// samples are always `duration: None`).
    pub fn new(tracks: Vec<TrackSpec>, target_secs: u32, window: usize) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "streaming ts-hls segmenter needs at least one track",
            ));
        }
        if window == 0 {
            return Err(Error::InvalidInput("window must be >= 1"));
        }
        for (i, a) in tracks.iter().enumerate() {
            if tracks[i + 1..].iter().any(|b| b.track_id == a.track_id) {
                return Err(Error::InvalidInput("duplicate track_id"));
            }
        }

        let anchor = choose_anchor(tracks.iter().map(|t| &t.config))?;
        let target_secs = target_secs.max(1);
        let target_ticks = target_ticks_for(target_secs, tracks[anchor].timescale.get());

        let tracks = tracks
            .into_iter()
            .map(|spec| StreamTrackState {
                spec,
                pending: Vec::new(),
                base_decode: 0,
                flush_clock: MediaClock::new(),
            })
            .collect();

        Ok(Self {
            tracks,
            anchor,
            target_secs,
            target_ticks,
            anchor_pending_dur: 0,
            deferred_cut: None,
            anchor_clock: MediaClock::new(),
            origin_secs: None,
            pending_discontinuity: false,
            cut_policy: SegmentCutPolicy::AnyRap,
            awaiting_idr: false,
            discontinuity_at_resync: false,
            dropped_leading_aus: 0,
            idr_wait: None,
            idr_fallback_active: false,
            finished: false,
            version: DEFAULT_HLS_VERSION,
            uri_prefix: String::from("seg"),
            window,
            window_segments: VecDeque::new(),
            continuity: TsContinuity::new(),
            total_segments: 0,
            discontinuity_sequence: 0,
            target_duration: 0,
            ready: VecDeque::new(),
        })
    }

    /// Build a streaming TS-HLS segmenter whose segment numbering starts
    /// from `next_sequence` (0-based, matching
    /// [`SegmentSummary::sequence`](crate::mux_stage::SegmentSummary::sequence)'s own
    /// convention) rather than [`Self::new`]'s implicit `0` — the classic-TS
    /// analogue of [`crate::ll_hls::LlHlsSegmenter::with_part_target_at`]: a
    /// segmenter rebuilt after a track-set change, or built against a
    /// `Trunk` that already holds segments from an earlier `SegmentWriter`,
    /// must resume numbering from there rather than renumbering from 0
    /// again — a restarted segmenter that numbers from 0 is exactly what a
    /// `Trunk`'s monotonic-`sequence_number` guard rejects every publish
    /// for.
    ///
    /// The usual constructor, [`Self::new`], starts at `0`.
    #[doc(hidden)]
    pub fn with_start_sequence(
        tracks: Vec<TrackSpec>,
        target_secs: u32,
        window: usize,
        next_sequence: u64,
    ) -> Result<Self> {
        let mut seg = Self::new(tracks, target_secs, window)?;
        seg.total_segments = next_sequence;
        Ok(seg)
    }

    /// Select the [`SegmentCutPolicy`] (default [`SegmentCutPolicy::AnyRap`]).
    ///
    /// With [`SegmentCutPolicy::IdrOnly`] the segmenter drops every sample
    /// until the anchor track delivers its first IDR picture, so the first
    /// segment starts on an IDR (Apple HLS Authoring Spec 7.4); the dropped
    /// anchor access units are counted by [`Self::dropped_leading_aus`].
    /// Only AVC/HEVC/VVC anchors carry IDR pictures: any other anchor codec
    /// keeps the sync-flag rule under either policy.
    ///
    /// Call before the first sample; a later call re-arms the wait
    /// only if nothing has been buffered or cut yet.
    #[must_use]
    pub fn with_cut_policy(mut self, policy: SegmentCutPolicy) -> Self {
        self.cut_policy = policy;
        let untouched =
            self.total_segments == 0 && self.tracks.iter().all(|t| t.pending.is_empty());
        self.awaiting_idr = policy == SegmentCutPolicy::IdrOnly && untouched;
        self
    }

    /// Bound the [`SegmentCutPolicy::IdrOnly`] wait for the first IDR: once
    /// `bound` of anchor media has been dropped without one, the segmenter
    /// falls back to any sync sample for the rest of the stream
    /// ([`Self::idr_fallback_active`]). Without this call it waits forever.
    #[must_use]
    pub fn with_idr_fallback(mut self, bound: core::time::Duration) -> Self {
        self.idr_wait = Some(crate::segment_cut::IdrWait::new(bound));
        self
    }

    /// Stop requiring an IDR: cut on any sync sample from now on. For a caller
    /// that measured the wait itself.
    pub fn fall_back_to_any_rap(&mut self) {
        self.idr_fallback_active = true;
    }

    /// `true` once [`SegmentCutPolicy::IdrOnly`] has fallen back to any sync
    /// sample (see [`Self::with_idr_fallback`]).
    pub fn idr_fallback_active(&self) -> bool {
        self.idr_fallback_active
    }

    /// The configured [`SegmentCutPolicy`].
    pub fn cut_policy(&self) -> SegmentCutPolicy {
        self.cut_policy
    }

    /// Anchor access units dropped because [`SegmentCutPolicy::IdrOnly`] was
    /// waiting for an IDR (session start, or after
    /// [`Self::mark_discontinuity`]). Always `0` under
    /// [`SegmentCutPolicy::AnyRap`].
    pub fn dropped_leading_aus(&self) -> u64 {
        self.dropped_leading_aus
    }

    fn anchor_nal_codec(&self) -> Option<NalCodec> {
        match self.tracks[self.anchor].spec.config {
            CodecConfig::Avc { .. } => Some(NalCodec::Avc),
            CodecConfig::Hevc { .. } => Some(NalCodec::Hevc),
            CodecConfig::Vvc { .. } => Some(NalCodec::Vvc),
            _ => None,
        }
    }

    fn opens_segment(&self, sample: &Sample) -> bool {
        match (self.cut_policy, self.anchor_nal_codec()) {
            (SegmentCutPolicy::IdrOnly, Some(codec)) if !self.idr_fallback_active => {
                access_unit_is_idr(codec, &sample.data)
            }
            _ => sample.flags.is_sync,
        }
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// The cut rule: when the anchor track reaches a sync sample past the
    /// target duration, the samples buffered so far are cut into a `.ts`
    /// segment *before* this sample is buffered, so the new keyframe opens the
    /// next segment. Non-anchor tracks are split by decode time. Feeding every
    /// sample of every track — interleaved so that no sample is fed before an
    /// earlier-or-equal-decode-time sample on another track — then finishing
    /// reproduces the segment boundaries and bytes a whole-input drive
    /// produces.
    ///
    /// Anchor progress is measured by [`MediaClock`]: each anchor sample's own
    /// `duration` when that is a real, non-zero span, else the `dts` delta
    /// from the previous anchor sample. Requiring a real `duration` used to be
    /// enforced here with a hard error, which rejected a shipped path outright
    /// — [`StreamingFlvDemux`](crate::flv_stream::StreamingFlvDemux) derives
    /// `duration` as the forward delta between FLV tag timestamps, so an RTMP
    /// publish's first sample (and any two tags sharing a timestamp) carries
    /// `Some(0)`, which the *other* three segmenters silently stalled on.
    ///
    /// [`Stage::feed`]'s sample body, leaving any cut segment on
    /// [`ready`](Self::ready) rather than returning it.
    fn push_inner(&mut self, track_id: TrackId, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        if self.awaiting_idr {
            if idx != self.anchor {
                return Ok(());
            }
            if !self.opens_segment(&sample) {
                self.dropped_leading_aus += 1;
                let timescale = self.tracks[self.anchor].spec.timescale;
                if let Some(wait) = self.idr_wait.as_mut()
                    && wait.observe_dropped(&sample, timescale)
                {
                    self.idr_fallback_active = true;
                }
                return Ok(());
            }
            self.awaiting_idr = false;
            self.complete_deferred()?;
            if !self.tracks[self.anchor].pending.is_empty() {
                self.cut_segment(false)?;
            }
            if self.discontinuity_at_resync {
                self.pending_discontinuity = self.total_segments > 0;
                self.discontinuity_at_resync = false;
            }
        }

        if idx == self.anchor
            && is_cut_point(
                !self.tracks[self.anchor].pending.is_empty(),
                self.opens_segment(&sample),
                self.anchor_pending_dur,
                self.target_ticks,
            )
        {
            self.cut_or_defer()?;
        }

        if self.tracks[idx].pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK {
            return Err(no_sync_sample_error());
        }

        if idx == self.anchor {
            if self.origin_secs.is_none() {
                self.origin_secs = Some(timeline_origin_secs(
                    sample.timing.and_then(|t| t.dts()),
                    self.tracks[self.anchor].spec.timescale,
                ));
            }
            self.anchor_pending_dur += self.anchor_clock.tick(&sample);
        }
        self.tracks[idx].pending.push(sample);
        self.try_complete_deferred()
    }

    /// Cut now, or hold the cut while an audio track still lags the boundary
    /// ([`DeferredCut`]).
    fn cut_or_defer(&mut self) -> Result<()> {
        self.complete_deferred()?;
        let anchor = self.anchor;
        let anchor_scale = self.tracks[anchor].spec.timescale.get() as u64;
        let next_start_ticks = self.tracks[anchor].base_decode + self.anchor_pending_dur;
        let next_start_secs = next_start_ticks as f64 / anchor_scale as f64;
        if !self.audio_lags(next_start_secs) {
            return self.cut_segment(false);
        }
        self.deferred_cut = Some(DeferredCut {
            anchor_split: self.tracks[anchor].pending.len(),
            closed_anchor_dur: self.anchor_pending_dur,
            next_start_secs,
        });
        self.anchor_pending_dur = 0;
        Ok(())
    }

    fn complete_deferred(&mut self) -> Result<()> {
        match self.deferred_cut.take() {
            Some(cut) => self.cut_segment_with(false, Some(cut)),
            None => Ok(()),
        }
    }

    fn try_complete_deferred(&mut self) -> Result<()> {
        let Some(cut) = self.deferred_cut else {
            return Ok(());
        };
        let scale = self.tracks[self.anchor].spec.timescale.get() as u64;
        let waited_out = self.anchor_pending_dur >= CATCH_UP_MAX_SECS.saturating_mul(scale);
        if waited_out || !self.audio_lags(cut.next_start_secs) {
            self.complete_deferred()?;
        }
        Ok(())
    }

    /// Whether some audio track that has delivered media holds none at or
    /// past `next_start_secs`, so cutting there would close its segment short.
    fn audio_lags(&self, next_start_secs: f64) -> bool {
        let origin_secs = self.origin_secs.unwrap_or(0.0);
        self.tracks.iter().enumerate().any(|(i, t)| {
            if i == self.anchor
                || t.spec.config.audio_sample_rate().is_none()
                || (t.base_decode == 0 && t.pending.is_empty())
            {
                return false;
            }
            let scale = t.spec.timescale;
            let mut acc = t.base_decode;
            let mut clock = MediaClock::resumed_at(t.flush_clock.last_dts());
            for s in &t.pending {
                if placement_secs(s, acc, scale, origin_secs) >= next_start_secs {
                    return false;
                }
                acc += clock.tick(s);
            }
            true
        })
    }

    /// Close the unit being built now: cut every track's pending samples into
    /// one segment without marking the segmenter finished, so the playlist
    /// keeps its live form.
    fn flush_inner(&mut self) -> Result<()> {
        self.complete_deferred()?;
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.cut_segment(true)?;
        }
        Ok(())
    }

    /// Finalize the trailing partial segment (call once at end-of-stream),
    /// leaving it on [`ready`](Self::ready) for [`Stage::poll`]. Every track's
    /// remaining pending samples are flushed into this one final segment
    /// regardless of decode time, and the segmenter is marked finished so
    /// [`Self::playlist`] appends `#EXT-X-ENDLIST`.
    fn finish_inner(&mut self) -> Result<()> {
        self.finished = true;
        self.complete_deferred()?;
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.cut_segment(true)?;
        }
        Ok(())
    }

    /// Mark the *next* segment cut as a media-timeline discontinuity
    /// (RFC 8216 §4.3.2.3) — call this when the caller detects a PID/PCR
    /// reset in the live source (e.g. an upstream PMT/PCR-PID change)
    /// between one push and the next.
    ///
    /// Under [`SegmentCutPolicy::IdrOnly`] the segmenter additionally drops
    /// samples until the anchor's next IDR, closes the pre-gap segment there,
    /// and flags the segment that IDR opens as discontinuous.
    pub fn mark_discontinuity(&mut self) {
        if self.cut_policy == SegmentCutPolicy::IdrOnly {
            self.awaiting_idr = true;
            self.discontinuity_at_resync = true;
        } else {
            self.pending_discontinuity = true;
        }
    }

    /// Register a track after construction (issue #624).
    ///
    /// A live [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux) resolves
    /// tracks incrementally — `DemuxEvent::TrackAdded` for one track can land
    /// after a segmenter has already been built and has already cut segments
    /// from an earlier-resolving track (e.g. video resolves and cuts a
    /// segment before audio's first frame parses). `add_track` lets that
    /// track join the segmenter mid-stream instead of forcing a rebuild that
    /// would silently drop it. Segments cut *before* `add_track` simply have
    /// no PES data for this track (spec-legal: a PMT may declare a track with
    /// zero samples in a given segment); every segment cut *after*
    /// `add_track` carries it correctly in both the PMT and the PES.
    ///
    /// # Anchor recomputation
    ///
    /// The anchor (the keyframe-cut-boundary track, normally "first video,
    /// else first track" — `choose_anchor`) was chosen from whatever track
    /// set existed at construction time and is deliberately **not**
    /// reconsidered on every `add_track`, or a video track added on segment
    /// 50 could retroactively invalidate the cut boundaries of the 49
    /// segments already emitted. The one case this method *does* recompute
    /// the anchor is: nothing has been cut yet (`total_segments == 0`), no
    /// track has any sample buffered yet (every `pending` is empty — so no
    /// cut/anchor decision has been made even provisionally), the current
    /// anchor is not already a video track (`CodecConfig::is_video`, internal), and
    /// the newly-added track *is* video. That exactly recovers the
    /// construction-time rule ("first video, else first track") for the case
    /// this issue targets: the segmenter had to be built audio-first because
    /// video hadn't resolved (or vice versa), so by the time
    /// `add_track(video)` runs, nothing has cut yet, hence it's safe. Every
    /// other case (anchor already video, new track isn't video, or anything
    /// has already been buffered/cut) keeps the existing anchor unchanged
    /// and simply adds the track for muxing.
    ///
    /// # PMT version and discontinuity
    ///
    /// Once any segment has been cut, the track set the stream carries
    /// changes, so the next segment's PMT is written with `version_number`
    /// incremented modulo 32 (ISO/IEC 13818-1 §2.4.4.9; issue #1699) and that
    /// segment is flagged discontinuous, because RFC 8216bis §4.4.4.3 requires
    /// `#EXT-X-DISCONTINUITY` when the number, type or identifiers of tracks
    /// change.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `spec.track_id` collides with an existing
    /// track.
    pub fn add_track(&mut self, spec: TrackSpec) -> Result<()> {
        if self.tracks.iter().any(|t| t.spec.track_id == spec.track_id) {
            return Err(Error::InvalidInput("add_track: duplicate track_id"));
        }

        let nothing_cut_or_buffered =
            self.total_segments == 0 && self.tracks.iter().all(|t| t.pending.is_empty());
        let current_anchor_is_video = self.tracks[self.anchor].spec.config.is_video();
        let new_is_video = spec.config.is_video();

        let new_index = self.tracks.len();
        self.tracks.push(StreamTrackState {
            spec,
            pending: Vec::new(),
            base_decode: 0,
            flush_clock: MediaClock::new(),
        });

        if self.total_segments > 0 {
            self.pending_discontinuity = true;
        }

        if nothing_cut_or_buffered && new_is_video && !current_anchor_is_video {
            self.anchor = new_index;
            self.target_ticks = target_ticks_for(
                self.target_secs,
                self.tracks[self.anchor].spec.timescale.get(),
            );
        }

        Ok(())
    }

    /// Update an existing track's spec mid-stream (issue #1425; RFC 8216
    /// section 4.3.4.3, Apple HLS Authoring Spec 2.11 / 8.14).
    ///
    /// Arms a discontinuity only when audio `sample_rate`, `channel_count` or
    /// the AAC AudioSpecificConfig bytes change, or the video `timescale` or
    /// `display` changes. SPS or bitrate-only changes do not. The stored spec
    /// is always refreshed.
    ///
    /// An unknown `track_id` returns [`UpdateTrackOutcome::Unknown`]; use
    /// [`Self::add_track`] for a new track.
    pub fn update_track(&mut self, spec: TrackSpec) -> Result<UpdateTrackOutcome> {
        let Some(idx) = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == spec.track_id)
        else {
            return Ok(UpdateTrackOutcome::Unknown);
        };

        let old = &self.tracks[idx].spec;

        let discontinuity = old.timescale != spec.timescale
            || old.display != spec.display
            || audio_discontinuity_trigger(&old.config, &spec.config);

        let old_timescale = self.tracks[idx].spec.timescale;
        self.tracks[idx].spec = spec;
        if discontinuity {
            if idx == self.anchor && self.tracks[idx].spec.timescale != old_timescale {
                self.target_ticks = target_ticks_for(
                    self.target_secs,
                    self.tracks[self.anchor].spec.timescale.get(),
                );
            }
            self.mark_discontinuity();
            Ok(UpdateTrackOutcome::Changed)
        } else {
            Ok(UpdateTrackOutcome::Unchanged)
        }
    }

    /// The current rolling media playlist: at most [`window`](Self::new)
    /// segments (the most recently cut), with `#EXT-X-MEDIA-SEQUENCE`
    /// advanced past every segment that has rolled out of the window and
    /// `#EXT-X-DISCONTINUITY-SEQUENCE` incremented once per discontinuous
    /// segment that has rolled *off* the window. RFC 8216 §6.2.1 defines a
    /// segment's Discontinuity Sequence Number as this header value plus the
    /// number of `#EXT-X-DISCONTINUITY` tags still visible in the current
    /// playlist preceding that segment's URI — so a discontinuous segment's
    /// own inline tag (rendered while it is still in the window) already
    /// accounts for its own boundary, and the header must only advance once
    /// that segment (and its tag) is gone, or every segment sharing the
    /// window with it would have its Discontinuity Sequence Number
    /// double-counted. No `#EXT-X-ENDLIST` until [`Stage::finish`] has been
    /// called.
    pub fn playlist(&self) -> crate::Result<String> {
        let segments: Vec<MediaSegment> = self
            .window_segments
            .iter()
            .map(|e| MediaSegment {
                uri: e.uri.clone(),
                duration: e.duration,
                discontinuous: e.discontinuous,
                parts: vec![],
                ..Default::default()
            })
            .collect();

        let msn = broadcast_hls::MediaSequence::new(
            self.total_segments - self.window_segments.len() as u64,
        );

        Ok(MediaPlaylist {
            version: self.version,
            target_duration: self.target_duration.max(1),
            msn,
            discontinuity_sequence: self.discontinuity_sequence,
            segments,
            endlist: self.finished,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
            ..Default::default()
        }
        .to_m3u8()?)
    }

    /// Cut samples into one `.ts` segment.
    ///
    /// When `final_cut` is `false` (a keyframe-triggered cut from
    /// a sample feed), the anchor's whole pending buffer closes out, and a
    /// non-anchor track's samples that decode-start at or after the new
    /// segment's start time stay pending for the next segment — the
    /// time-based split for non-anchor tracks. When `final_cut` is `true`
    /// (from [`Stage::finish`] or [`Stage::feed`]'s flush), every track's
    /// entire pending buffer is flushed regardless of decode time, so the
    /// trailing tail of every track lands in the closing segment.
    ///
    /// The cut segment is pushed onto [`ready`](Self::ready), the single queue
    /// both the inherent API and [`Stage::poll`] drain.
    fn cut_segment(&mut self, final_cut: bool) -> Result<()> {
        self.cut_segment_with(final_cut, None)
    }

    fn cut_segment_with(&mut self, final_cut: bool, deferred: Option<DeferredCut>) -> Result<()> {
        let closed_anchor_dur = deferred.map_or(self.anchor_pending_dur, |d| d.closed_anchor_dur);
        let anchor = self.anchor;
        let anchor_scale = self.tracks[anchor].spec.timescale.get() as u64;
        let origin_secs = self.origin_secs.unwrap_or(0.0);
        // The new segment's start time = this track's cumulative decode time
        // so far (already-flushed ticks + the pending anchor buffer) — the
        // The segment's start time: cumulative decode ticks so far.
        let next_start_ticks = self.tracks[anchor].base_decode + closed_anchor_dur;
        let next_start_secs = next_start_ticks as f64 / anchor_scale as f64;

        // Per-track split point: everything pending for the anchor (and for
        // every track on the final cut); time-partitioned for non-anchor
        // tracks on a regular keyframe-triggered cut.
        let split_at: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if final_cut {
                    return t.pending.len();
                }
                if i == anchor {
                    return deferred.map_or(t.pending.len(), |d| d.anchor_split);
                }
                let scale = t.spec.timescale;
                let mut acc = t.base_decode;
                // `MediaClock`/`placement_secs` place a given sample —
                // including a timestamped, duration-less section sample — by
                // its own decode time.
                let mut clock = MediaClock::resumed_at(t.flush_clock.last_dts());
                for (j, s) in t.pending.iter().enumerate() {
                    let start_secs = placement_secs(s, acc, scale, origin_secs);
                    if start_secs >= next_start_secs {
                        return j;
                    }
                    acc += clock.tick(s);
                }
                t.pending.len()
            })
            .collect();

        // Ephemeral `Track`s carrying only the spec (the mux reads
        // `track.spec` for PID/stream_type planning; samples are passed
        // separately as borrowed slices below).
        let mux_tracks: Vec<Track> = self
            .tracks
            .iter()
            .map(|t| Track::new(t.spec.clone(), Vec::new()))
            .collect();
        let sample_slices: Vec<&[Sample]> = self
            .tracks
            .iter()
            .zip(&split_at)
            .map(|(t, &n)| &t.pending[..n])
            .collect();
        let base_dts: Vec<u64> = self.tracks.iter().map(|t| t.base_decode).collect();
        let bytes =
            mux_tracks_at_continuing(&mux_tracks, &sample_slices, &base_dts, &mut self.continuity)?;

        let (duration, ceil_secs) =
            segment_duration(closed_anchor_dur, self.tracks[anchor].spec.timescale);
        if ceil_secs > self.target_duration {
            self.target_duration = ceil_secs;
        }

        let discontinuous = self.pending_discontinuity;
        self.pending_discontinuity = false;

        // Drop the flushed prefix of each track's pending buffer and advance
        // its base_decode past it.
        let mut parts = Vec::with_capacity(self.tracks.len());
        for (t, &n) in self.tracks.iter_mut().zip(&split_at) {
            let clock = &mut t.flush_clock;
            let dur: u64 = t.pending[..n].iter().map(|s| clock.tick(s)).sum();
            if n > 0 {
                parts.push(TrackPart {
                    track: t.spec.track_id,
                    start: t.base_decode,
                    duration: dur,
                    timescale: t.spec.timescale.get(),
                    bytes: t.pending[..n]
                        .iter()
                        .map(|s| u64::try_from(s.data.len()).unwrap_or(u64::MAX))
                        .sum(),
                    samples: u32::try_from(n).unwrap_or(u32::MAX),
                });
            }
            t.base_decode += dur;
            t.pending.drain(..n);
        }
        if deferred.is_none() {
            self.anchor_pending_dur = 0;
        }

        let sequence = self.total_segments;
        let uri = format!("{}{}.ts", self.uri_prefix, sequence);
        self.window_segments.push_back(WindowEntry {
            uri: uri.clone(),
            // A `Duration` is always finite and non-negative.
            duration: DecimalSeconds::from_duration(duration),
            discontinuous,
        });
        self.total_segments += 1;
        while self.window_segments.len() > self.window {
            if let Some(dropped) = self.window_segments.pop_front()
                && dropped.discontinuous
            {
                self.discontinuity_sequence += 1;
            }
        }

        let summary = SegmentSummary {
            track: self.tracks[anchor].spec.track_id,
            sequence,
            start: next_start_ticks,
            duration: closed_anchor_dur,
            timescale: self.tracks[anchor].spec.timescale.get(),
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            independent: true,
            discontinuous,
        };
        self.ready
            .push_back(MuxOutput::Segment(Box::new(SegmentOut {
                bytes: bytes.into(),
                summary,
                tracks: parts,
            })));
        Ok(())
    }
}

/// [`Stage`] adoption (media plane step 2e-2 / Phase 4): the segmenter takes
/// the muxer vocabulary — a track announcement or update, one sample, a
/// timeline discontinuity or an explicit flush — and emits
/// [`MuxOutput::Segment`] per cut (a filled [`SegmentSummary`] plus the
/// segment's bytes).
///
/// Every cut lands on the private `ready` queue [`poll`](Stage::poll) drains
/// and *only* there (issue R2), so a segment is retrievable exactly once.
impl Stage for StreamingTsHlsSegmenter {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => {
                let spec = *spec;
                if self.tracks.iter().any(|t| t.spec.track_id == spec.track_id) {
                    self.update_track(spec).map(|_| ())
                } else {
                    self.add_track(spec)
                }
            }
            MuxInput::Sample { track, sample } => self.push_inner(track, sample),
            MuxInput::Discontinuity { .. } => {
                self.mark_discontinuity();
                Ok(())
            }
            MuxInput::Flush => self.flush_inner(),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.finish_inner()
    }

    /// `saturated` once any track holds [`MAX_PENDING_SAMPLES_PER_TRACK`]
    /// un-cut samples — the same bound (and reasoning) as
    /// [`Segmenter`](crate::segmenter::Segmenter)'s: past it
    /// [`feed`](Stage::feed) errors rather than buffering a stream that never
    /// produces the sync sample a `.ts` segment must open on.
    fn demand(&self) -> Demand {
        Demand::saturated_if(
            self.tracks
                .iter()
                .any(|t| t.pending.len() >= MAX_PENDING_SAMPLES_PER_TRACK),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{CodecConfig, DataCarriage, DataIdentity};

    fn feed_sample(seg: &mut StreamingTsHlsSegmenter, track: TrackId, sample: Sample) {
        Stage::feed(seg, MuxInput::Sample { track, sample }, Timestamp::ZERO).expect("feed sample");
    }

    fn drain_segments(seg: &mut StreamingTsHlsSegmenter) -> Vec<SegmentOut> {
        let mut out = Vec::new();
        while let Some(item) = Stage::poll(seg) {
            if let MuxOutput::Segment(segment) = item {
                out.push(*segment);
            }
        }
        out
    }

    #[test]
    fn cut_point_needs_pending_media_a_sync_sample_and_the_target() {
        // The cut rule every segmenter shares: a sync sample past the target,
        // with media already buffered for the segment it closes.
        assert!(is_cut_point(true, true, 2, 2), "sync at the target cuts");
        assert!(is_cut_point(true, true, 3, 2), "sync past the target cuts");
        assert!(
            !is_cut_point(true, true, 1, 2),
            "a sync sample before the target does not cut"
        );
        assert!(
            !is_cut_point(false, true, 1000, 1),
            "the first sample opens a segment, it never closes one"
        );
        assert!(
            !is_cut_point(true, false, 1000, 1),
            "a non-sync sample never opens a segment"
        );
    }

    // ── Issue B9: a section-carried anchor must not stall the segmenter ────
    //
    // A section-carried track (e.g. SCTE-35 `splice_info_section`, carried
    // directly on its own PID with no PES header — ISO/IEC 13818-1 §2.4.4)
    // always has `duration: None` (media plane step 2c: never fabricated).
    // Before this fix, `choose_anchor` fell back to track index 0 whenever
    // no video track was present, so a track set like "SCTE-35 first, audio
    // second" picked the SCTE-35 track as the anchor: its buffered duration
    // could never advance, `push` never returned a segment, and every
    // track's `pending` buffer grew without bound.
    use broadcast_common::timescale::Timescale;
    use isobmff::mp4esds::{
        DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    };

    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    fn dummy_esds() -> EsdsBox {
        EsdsBox::new(ESDescriptor::new(
            1,
            0,
            Some(DecoderConfigDescriptor::new(
                0x40,
                0x05,
                false,
                0,
                0,
                0,
                Some(DecoderSpecificInfo::new(vec![0x12, 0x10])),
            )),
            Some(SLConfigDescriptor::predefined_two()),
        ))
    }

    /// A section-carried SCTE-35 track spec (ISO/IEC 13818-1 Table 2-34
    /// stream_type 0x86, ANSI/SCTE 35): permanently `duration: None`, so it
    /// must never be picked as the segmentation anchor.
    fn scte35_track(track_id: TrackId) -> TrackSpec {
        TrackSpec::new(
            track_id,
            timescale(90_000),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 0x86,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Sections,
                },
            },
        )
    }

    /// A `splice_info_section`-shaped placeholder sample: no timestamp, no
    /// duration — exactly what `TsDemux`/`StreamingTsDemux` emit for a
    /// section-carried track (never fabricated).
    fn section_sample() -> Sample {
        Sample {
            data: vec![0xFCu8, 0x30, 0x11].into(),
            timing: None,
            flags: crate::ir::SampleFlags::SYNC,
            provenance: None,
            display: None,
            protection: None,
        }
    }

    fn aac_track(track_id: TrackId, timescale: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            broadcast_common::timescale::Timescale::try_new(timescale).expect("timescale"),
            CodecConfig::Aac {
                esds: dummy_esds(),
                channel_count: 2,
                sample_rate: timescale,
                sample_size: 16,
            },
        )
    }

    /// An AAC-shaped audio sample: always a sync sample (mirrors
    /// `Sample::from_raw`), with a real, caller-supplied duration.
    fn audio_sample(duration: u32) -> Sample {
        Sample::from_raw(vec![0u8; 4], None, None, Some(duration)).expect("valid timing")
    }

    #[test]
    fn streaming_segmenter_advances_past_a_section_track_anchor_and_cuts_real_segments() {
        const SCTE_ID: u32 = 10;
        const AUDIO_ID: u32 = 20;
        const TIMESCALE: u32 = 1000;
        const TARGET_SECS: u32 = 1; // target_ticks = 1000

        // Section track first, no video — the exact routine DVB/ad-insertion
        // shape that used to stall forever.
        let mut seg = StreamingTsHlsSegmenter::new(
            vec![
                scte35_track(TrackId::new(SCTE_ID)),
                aac_track(TrackId::new(AUDIO_ID), TIMESCALE),
            ],
            TARGET_SECS,
            usize::MAX,
        )
        .expect("construct: audio is anchor-capable even though it isn't first");

        // A couple of section samples up front (duration: None) must not
        // prevent the audio anchor from advancing.
        feed_sample(&mut seg, TrackId::new(SCTE_ID), section_sample());

        // 12 audio samples of 200 ticks each: a cut fires once buffered
        // reaches the 1000-tick target on the 6th and 11th pushes. Cuts are
        // queued, not returned inline (issue R2) — drained via `take_ready`.
        for _ in 0..12 {
            feed_sample(&mut seg, TrackId::new(AUDIO_ID), audio_sample(200));
        }
        let cuts = drain_segments(&mut seg);

        assert_eq!(
            cuts.len(),
            2,
            "the audio anchor must have advanced past the 1000-tick target twice \
             — before the fix this was 0 (the segmenter stalled forever)"
        );
        for (i, c) in cuts.iter().enumerate() {
            assert!(
                !c.bytes.is_empty(),
                "cut segment {i} must carry real TS bytes"
            );
            assert!(
                c.summary.duration > 0,
                "cut segment {i} must have a positive duration"
            );
        }

        // Every emitted segment is a byte-oracle-verifiable real TS mux —
        // decode the first one and confirm it actually carries the AAC PES.
        let demuxed = crate::test_media::media_from_ts(&cuts[0].bytes)
            .expect("cut segment must be a valid TS stream");
        assert!(
            demuxed
                .tracks
                .iter()
                .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. }) && !t.samples.is_empty()),
            "cut segment must carry the audio track's samples, not just PSI"
        );

        // The pending buffer stayed bounded (2 audio samples left over after
        // 12 pushes and 2 cuts) — before the fix this would have grown to
        // 12 (audio) with the section track's backlog also never draining.
        let audio_track_state = seg
            .tracks
            .iter()
            .find(|t| t.spec.track_id == TrackId::new(AUDIO_ID))
            .expect("audio track state");
        assert_eq!(
            audio_track_state.pending.len(),
            2,
            "pending must be bounded by the cut cadence, not grow without bound"
        );
    }

    #[test]
    fn streaming_segmenter_construction_errors_loudly_with_no_anchorable_track() {
        // Only section/opaque tracks, no video/audio: there is no track
        // whose duration can ever advance a cut boundary. Prefer a loud
        // construction error over a segmenter that silently never cuts.
        let result = StreamingTsHlsSegmenter::new(
            vec![
                scte35_track(TrackId::new(10)),
                scte35_track(TrackId::new(11)),
            ],
            1,
            usize::MAX,
        );
        match result {
            Err(Error::InvalidInput(_)) => {}
            Err(other) => panic!("expected InvalidInput, got a different error: {other:?}"),
            Ok(_) => panic!("a track set of only section-carried tracks must not construct"),
        }
    }

    /// A `Stage`-driven segmenter must hand the trailing segment `finish`
    /// cuts back through `Stage::poll`, never lose it.
    #[test]
    fn finish_bare_call_does_not_lose_the_trailing_segment_to_a_poll_driver() {
        const AUDIO_ID: u32 = 1;
        const TIMESCALE: u32 = 1000;

        let mut seg = StreamingTsHlsSegmenter::new(
            vec![aac_track(TrackId::new(AUDIO_ID), TIMESCALE)],
            1000,
            usize::MAX,
        )
        .expect("construct");

        // One sample only: not enough to cross the (huge) cut target, so
        // nothing is cut yet and the only segment `Stage::poll` will ever see
        // is the trailing one `finish` cuts below.
        feed_sample(&mut seg, TrackId::new(AUDIO_ID), audio_sample(200));
        assert!(
            Stage::poll(&mut seg).is_none(),
            "no cut yet: nothing should be ready before finish"
        );

        Stage::finish(&mut seg).expect("finish");

        assert!(
            matches!(Stage::poll(&mut seg), Some(MuxOutput::Segment(_))),
            "the trailing segment cut by finish must be retrievable via Stage::poll"
        );
    }

    /// [`StreamingTsHlsSegmenter::with_start_sequence`] must number the
    /// first segment it cuts from the seed, not from `0` — the seam
    /// `multimux::source::segment` uses to resume TS numbering across an
    /// `add_track` failure/rebuild instead of renumbering from `0` again
    /// (which a `Trunk`'s monotonic `sequence_number` guard would then
    /// reject forever). `Self::new` (seed implicitly `0`) is exercised by
    /// every other test in this module; this is the seeded constructor's
    /// own coverage.
    #[test]
    fn with_start_sequence_numbers_the_first_cut_segment_from_the_seed() {
        const AUDIO_ID: u32 = 1;
        const TIMESCALE: u32 = 1000;
        const SEED: u64 = 41;

        let mut seg = StreamingTsHlsSegmenter::with_start_sequence(
            vec![aac_track(TrackId::new(AUDIO_ID), TIMESCALE)],
            1,
            usize::MAX,
            SEED,
        )
        .expect("construct");

        // Two 1000-tick (1s) samples cross the 1s target on the second push.
        feed_sample(&mut seg, TrackId::new(AUDIO_ID), audio_sample(1000));
        feed_sample(&mut seg, TrackId::new(AUDIO_ID), audio_sample(1000));
        Stage::finish(&mut seg).expect("finish");

        let cut = drain_segments(&mut seg);
        assert_eq!(
            cut.iter().map(|s| s.summary.sequence).collect::<Vec<_>>(),
            vec![SEED, SEED + 1],
            "segments cut by a seeded segmenter must number from the seed, not from 0"
        );
    }
    fn avc_spec(track_id: u32) -> TrackSpec {
        use isobmff::nalu_types::{AvcPps, AvcSps};
        let record = isobmff::avc_config::AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0x42,
            profile_compatibility: 0,
            level_indication: 0x1E,
            length_size_minus_one: 3,
            sps: vec![AvcSps(vec![0x67, 0x42, 0xc0, 0x1e, 0xd9])],
            pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        };
        TrackSpec::new(
            TrackId::new(track_id),
            timescale(90_000),
            CodecConfig::Avc {
                config: isobmff::avc_config::AVCConfigurationBox::new(record),
                width: 320,
                height: 240,
            },
        )
    }

    fn timed_sample(dts: i64, duration: Option<u32>, sync: bool) -> Sample {
        Sample::new(vec![0u8; 4], Some(dts), Some(dts), duration, sync).expect("valid timing")
    }

    #[test]
    fn timeline_origin_is_the_anchor_first_decode_time() {
        assert_eq!(timeline_origin_secs(Some(900_000), timescale(90_000)), 10.0);
        assert_eq!(timeline_origin_secs(None, timescale(90_000)), 0.0);
        assert_eq!(timeline_origin_secs(Some(-5), timescale(90_000)), 0.0);
        let s = timed_sample(900_000, None, false);
        assert_eq!(placement_secs(&s, 0, timescale(90_000), 10.0), 0.0);
    }

    #[test]
    fn duration_less_dts_samples_partition_by_their_own_dts() {
        // A timestamped, duration-less sample (a section-carried SCTE-35
        // splice, say) is placed by its own `dts`, not by an accumulator that
        // can never advance: video starts at 10 s with one 1 s sample each,
        // the data samples sit at 10.0/10.5/11.0/11.5 s, so the 1 s segments
        // take two data samples each into the first two and none into the
        // last two.
        let video = avc_spec(1);
        let data = scte35_track(TrackId::new(2));
        let mut seg =
            StreamingTsHlsSegmenter::new(vec![video, data], 1, usize::MAX).expect("construct");
        let video_dts = [900_000i64, 990_000, 1_080_000, 1_170_000];
        let data: Vec<Sample> = [900_000, 945_000, 990_000, 1_035_000]
            .iter()
            .map(|&dts| timed_sample(dts, None, true))
            .collect();
        let mut order: Vec<(TrackId, Sample)> = video_dts
            .iter()
            .map(|&dts| (TrackId::new(1), timed_sample(dts, Some(90_000), true)))
            .collect();
        order.extend(data.into_iter().map(|sample| (TrackId::new(2), sample)));
        order.sort_by_key(|(track, sample)| {
            (
                sample.timing.and_then(|t| t.dts()).unwrap_or(0),
                track.value(),
            )
        });

        let mut counts: Vec<usize> = Vec::new();
        for (track, sample) in order {
            let before = seg.tracks[1].pending.len();
            feed_sample(&mut seg, track, sample);
            if seg.total_segments as usize > counts.len() {
                counts.push(before - seg.tracks[1].pending.len());
            }
        }
        let before = seg.tracks[1].pending.len();
        Stage::finish(&mut seg).expect("finish");
        if seg.total_segments as usize > counts.len() {
            counts.push(before - seg.tracks[1].pending.len());
        }
        assert_eq!(counts, vec![2, 2, 0, 0]);
    }

    #[test]
    fn segment_duration_rounds_half_up_and_ceils_seconds() {
        let (duration, ceil_secs) = segment_duration(5, timescale(90_000));
        assert_eq!(duration, Duration::from_nanos(55_556));
        assert_eq!(ceil_secs, 1);
        assert_eq!(
            segment_duration(90_000, timescale(90_000)),
            (Duration::from_secs(1), 1)
        );
    }
}
