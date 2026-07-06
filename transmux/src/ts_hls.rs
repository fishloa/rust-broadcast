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
//! [`TsHlsPackager`] segments the hub [`Media`] IR at keyframe boundaries on the
//! **anchor track** (the first video track, else the first track), cutting a new
//! segment on the first sync sample at or past the target duration — mirroring
//! [`Segmenter`](crate::segmenter::Segmenter)'s CMAF rule so every video segment
//! begins on a random-access point. Every track's samples are partitioned across
//! the segments by decode time, so the concatenation of all segments carries the
//! full input (no sample dropped, duplicated, or reordered). Each segment is then
//! muxed via the shared [`crate::ts_mux`] machinery, re-emitting PAT + PMT at its
//! start (ISO/IEC 13818-1 §2.4.4 PSI repetition — a receiver joining mid-stream
//! must find the PSI at each segment boundary).
//!
//! The playlist is a VOD media playlist: `#EXTM3U`, `#EXT-X-VERSION`,
//! `#EXT-X-TARGETDURATION` (≥ every `#EXTINF`), `#EXT-X-MEDIA-SEQUENCE`, one
//! `#EXTINF` + `.ts` URI per segment, and a trailing `#EXT-X-ENDLIST`
//! (RFC 8216 §4.3.3). `#EXT-X-DISCONTINUITY` (RFC 8216 §4.3.4.3) and
//! `#EXT-X-DISCONTINUITY-SEQUENCE` (RFC 8216 §4.3.3.3) are forwarded from the
//! underlying [`MediaPlaylist`] when the [`MediaSegment::discontinuous`] flag
//! is set; this packager itself produces a single continuous timeline from one
//! contiguous IR, so it does not set the flag on any generated segment.
//!
//! # Streaming (live) input — [`StreamingTsHlsSegmenter`]
//!
//! [`TsHlsPackager::package`] is batch: it needs the whole [`Media`] up front.
//! For a live, unbounded feed there is no whole-input IR — [`StreamingTsHlsSegmenter`]
//! is the incremental analogue, mirroring [`Segmenter`](crate::segmenter::Segmenter)'s
//! CMAF push/flush model: [`StreamingTsHlsSegmenter::push`] buffers one coded
//! sample at a time and returns a finished `.ts` [`TsSegment`] whenever the
//! anchor track crosses a keyframe past the target duration;
//! [`StreamingTsHlsSegmenter::finish`] flushes the trailing partial segment;
//! [`StreamingTsHlsSegmenter::playlist`] renders a rolling media playlist over
//! a configurable sliding window, advancing `#EXT-X-MEDIA-SEQUENCE` as older
//! segments roll off and `#EXT-X-DISCONTINUITY-SEQUENCE` to the window's
//! current leading segment's absolute discontinuity count (RFC 8216
//! §4.3.3.3 — a segment carrying `#EXT-X-DISCONTINUITY` belongs to the
//! incremented sequence from the moment it becomes the playlist's first
//! segment, not once it later rolls off in turn), and omitting
//! `#EXT-X-ENDLIST` until `finish` has been called. Both types share the same
//! anchor-selection, cut-decision, and duration-accounting logic (this
//! module's private `choose_anchor`/`is_cut_point`/`segment_duration_secs`
//! helpers) so they can never drift apart.
//!
//! # Spec
//!
//! - **HLS media playlist**: RFC 8216 §4.1 / §4.3.2 (`#EXTINF`) / §4.3.3
//!   (`#EXT-X-TARGETDURATION`, `#EXT-X-MEDIA-SEQUENCE`, `#EXT-X-ENDLIST`).
//! - **TS segment constraints (PAT/PMT at each segment start)**:
//!   ISO/IEC 13818-1 §2.4.4 — PSI is periodically repeated so any access point
//!   is self-describing.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::Package;

use crate::error::{Error, Result};
use crate::hls::{MediaPlaylist, MediaSegment};
use crate::media::{Media, Track};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::ts_mux::mux_tracks_at;

/// Default `#EXT-X-VERSION` for a classic (TS-segment) media playlist. Version 3
/// is the floor for floating-point `#EXTINF` durations (RFC 8216 §7).
const DEFAULT_HLS_VERSION: u8 = 3;

/// The output of [`TsHlsPackager`]: the `.ts` media segments plus the media
/// playlist referencing them.
///
/// `segments[i]` is the whole-packet MPEG-2 TS bytes of the segment named by the
/// *i*-th `#EXTINF`/URI pair in `playlist`; the URIs are `{prefix}{i}.ts`.
#[derive(Debug, Clone)]
pub struct TsHlsOutput {
    /// The `.ts` media segments, in playlist order. Each is a whole number of
    /// 188-byte TS packets and opens with a PAT + PMT.
    pub segments: Vec<Vec<u8>>,
    /// The RFC 8216 media playlist (`#EXTM3U` … `#EXT-X-ENDLIST`) referencing
    /// `segments` by URI.
    pub playlist: String,
}

/// Package a hub [`Media`] IR into classic HLS: keyframe-aligned MPEG-2 TS
/// `.ts` media segments + an RFC 8216 media playlist.
///
/// Construct with [`TsHlsPackager::new`] or [`TsHlsPackager::default`], then call
/// [`Package::package`]. The target segment duration is a whole number of seconds
/// (integer arithmetic, `no_std`-friendly).
///
/// ```
/// use broadcast_common::{Package, Unpackage};
/// use transmux::{TsDemux, TsHlsPackager};
/// # fn ts_bytes() -> Vec<u8> { std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts")).unwrap() }
/// let ir = TsDemux::new().unpackage(&ts_bytes()).unwrap();
/// let out = TsHlsPackager::new(1).package(&ir).unwrap();
/// assert!(out.playlist.starts_with("#EXTM3U"));
/// assert!(out.segments.iter().all(|s| s.len() % 188 == 0));
/// ```
#[derive(Debug, Clone)]
pub struct TsHlsPackager {
    /// Target segment duration in whole seconds. Segments are cut on the first
    /// anchor-track keyframe at or past this many seconds of buffered anchor
    /// media (so the actual duration may be slightly longer, keyframe-aligned).
    pub target_secs: u32,
    /// `#EXT-X-VERSION` written to the playlist.
    pub version: u8,
    /// `#EXT-X-MEDIA-SEQUENCE` of the first segment.
    pub media_sequence: u64,
    /// URI prefix for the generated `.ts` segment entries (`{prefix}{i}.ts`).
    pub uri_prefix: String,
}

impl Default for TsHlsPackager {
    fn default() -> Self {
        Self::new(6)
    }
}

impl TsHlsPackager {
    /// Create a packager targeting `target_secs`-second segments (clamped to at
    /// least 1 second), with the default HLS version, media sequence 0, and the
    /// `"seg"` URI prefix.
    pub fn new(target_secs: u32) -> Self {
        Self {
            target_secs: target_secs.max(1),
            version: DEFAULT_HLS_VERSION,
            media_sequence: 0,
            uri_prefix: String::from("seg"),
        }
    }
}

/// One segment's per-track sample ranges, expressed as `[start, end)` indices
/// into each track's `samples`. `ranges[t]` is the half-open range for
/// `tracks[t]`; an empty range means that track contributes no sample here.
struct SegmentRanges {
    ranges: Vec<core::ops::Range<usize>>,
}

impl Package for TsHlsPackager {
    type Media = Media;
    type Output = TsHlsOutput;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<TsHlsOutput> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }

        // Choose the anchor track: first video (AVC), else the first track —
        // mirrors Segmenter's anchor selection.
        let anchor = choose_anchor(media.tracks.iter().map(|t| &t.spec.config));

        let target_ticks = self.anchor_target_ticks(&media.tracks[anchor]);
        let boundaries = anchor_segment_boundaries(&media.tracks[anchor].samples, target_ticks);
        let segments = partition_tracks(&media.tracks, anchor, &boundaries);

        // Mux each segment independently: each re-emits PAT/PMT then its PES.
        let mut ts_segments: Vec<Vec<u8>> = Vec::with_capacity(segments.len());
        let mut playlist_segments: Vec<MediaSegment> = Vec::with_capacity(segments.len());
        let mut target_duration: u32 = 0;
        for (i, seg) in segments.iter().enumerate() {
            let sample_slices: Vec<&[Sample]> = media
                .tracks
                .iter()
                .zip(&seg.ranges)
                .map(|(t, r)| &t.samples[r.clone()])
                .collect();
            // Base DTS per track = cumulative duration of all samples before this
            // segment's start, so the segment continues the previous timeline and
            // the concatenation forms one monotonic DTS/PTS timeline.
            let base_dts: Vec<u64> = media
                .tracks
                .iter()
                .zip(&seg.ranges)
                .map(|(t, r)| t.samples[..r.start].iter().map(|s| s.duration as u64).sum())
                .collect();
            let bytes = mux_tracks_at(&media.tracks, &sample_slices, &base_dts)?;
            ts_segments.push(bytes);

            // Segment duration = the anchor track's buffered duration (seconds).
            let anchor_ticks: u64 = media.tracks[anchor].samples[seg.ranges[anchor].clone()]
                .iter()
                .map(|s| s.duration as u64)
                .sum();
            let ts_scale = media.tracks[anchor].spec.timescale.max(1) as u64;
            // #EXT-X-TARGETDURATION is an integer ≥ every #EXTINF (RFC 8216
            // §4.3.3.1: the rounded max segment duration).
            let (duration, ceil_secs) = segment_duration_secs(anchor_ticks, ts_scale);
            if ceil_secs > target_duration {
                target_duration = ceil_secs;
            }
            playlist_segments.push(MediaSegment {
                uri: format!("{}{}.ts", self.uri_prefix, i),
                duration,
                discontinuous: false,
                parts: vec![],
            });
        }

        let playlist = MediaPlaylist {
            version: self.version,
            target_duration: target_duration.max(1),
            media_sequence: self.media_sequence,
            discontinuity_sequence: 0,
            segments: playlist_segments,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
        }
        .to_m3u8();

        Ok(TsHlsOutput {
            segments: ts_segments,
            playlist,
        })
    }
}

impl TsHlsPackager {
    /// Convert the configured `target_secs` into anchor-track timescale ticks
    /// (never zero, so a segment can always close).
    fn anchor_target_ticks(&self, anchor: &Track) -> u64 {
        target_ticks_for(self.target_secs, anchor.spec.timescale)
    }
}

// ── Shared segment-cutting logic (batch + streaming) ────────────────────────
//
// [`TsHlsPackager`] (batch) and [`StreamingTsHlsSegmenter`] (incremental) both
// cut segments on the same anchor-track keyframe rule and share the exact same
// duration/anchor-selection arithmetic below, so the two paths can never
// silently drift apart (issue #571).

/// Choose the anchor track index used for segment-cut boundaries: the first
/// video (AVC) track, else the first track — mirrors
/// [`Segmenter`](crate::segmenter::Segmenter)'s anchor selection.
fn choose_anchor<'a, I>(configs: I) -> usize
where
    I: Iterator<Item = &'a CodecConfig>,
{
    configs
        .enumerate()
        .find(|(_, c)| matches!(c, CodecConfig::Avc { .. }))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Convert a whole-seconds target duration into anchor-track timescale ticks
/// (never zero, so a segment can always close).
fn target_ticks_for(target_secs: u32, ts_scale: u32) -> u64 {
    (target_secs.max(1) as u64) * (ts_scale.max(1) as u64)
}

/// Segment duration in seconds, plus the RFC 8216 `#EXT-X-TARGETDURATION`
/// ceiling (whole seconds, rounded up) for `anchor_ticks` of buffered anchor
/// media at `ts_scale` ticks/second.
fn segment_duration_secs(anchor_ticks: u64, ts_scale: u64) -> (f64, u32) {
    let ts_scale = ts_scale.max(1);
    let duration = anchor_ticks as f64 / ts_scale as f64;
    let ceil_secs = anchor_ticks.div_ceil(ts_scale) as u32;
    (duration, ceil_secs)
}

/// True when the anchor should be cut *before* buffering the incoming sample:
/// the current segment already has content (`has_pending`), the sample is a
/// sync sample, and the buffered duration has reached the target. Shared by
/// the batch boundary scan ([`anchor_segment_boundaries`]) and
/// [`StreamingTsHlsSegmenter::push`]'s incremental cut decision.
fn is_cut_point(has_pending: bool, is_sync: bool, buffered_ticks: u64, target_ticks: u64) -> bool {
    has_pending && is_sync && buffered_ticks >= target_ticks
}

/// Compute the anchor-track sample indices at which a new segment starts.
///
/// Always includes `0`. A cut is made *before* a sync sample once the buffered
/// anchor duration since the last cut has reached `target_ticks` — the same rule
/// as [`Segmenter`](crate::segmenter::Segmenter), so every segment begins on a
/// keyframe (random-access point). Returns the ascending list of start indices.
fn anchor_segment_boundaries(samples: &[Sample], target_ticks: u64) -> Vec<usize> {
    let mut starts = vec![0usize];
    if samples.is_empty() {
        return starts;
    }
    let mut buffered: u64 = 0;
    for (i, s) in samples.iter().enumerate() {
        // Cut before this sample when it is a keyframe past the target and it is
        // not the very first sample (the leading segment already starts at 0).
        if is_cut_point(i > 0, s.is_sync, buffered, target_ticks) {
            starts.push(i);
            buffered = 0;
        }
        buffered += s.duration as u64;
    }
    starts
}

/// Partition every track's samples into per-segment index ranges.
///
/// The anchor track is split exactly at `anchor_boundaries`. Non-anchor tracks
/// (e.g. audio) are split by decode time: each sample is assigned to the segment
/// whose anchor-time window `[seg_start_time, next_seg_start_time)` contains the
/// sample's decode-start time, computed in seconds so tracks with different
/// timescales align. Every sample lands in exactly one segment, in order, so a
/// concatenation of the segments reproduces each track's full sample list.
fn partition_tracks(
    tracks: &[Track],
    anchor: usize,
    anchor_boundaries: &[usize],
) -> Vec<SegmentRanges> {
    let n_segs = anchor_boundaries.len();
    let anchor_samples = &tracks[anchor].samples;
    let anchor_scale = tracks[anchor].spec.timescale.max(1) as u64;

    // Segment start times (seconds) on the anchor timeline; the last segment
    // runs to +∞ so it captures any trailing tail on longer tracks.
    let mut start_times: Vec<f64> = Vec::with_capacity(n_segs);
    {
        let mut acc: u64 = 0;
        let mut cursor = 0usize;
        for &b in anchor_boundaries {
            while cursor < b {
                acc += anchor_samples[cursor].duration as u64;
                cursor += 1;
            }
            start_times.push(acc as f64 / anchor_scale as f64);
        }
    }

    let mut out: Vec<SegmentRanges> = (0..n_segs)
        .map(|_| SegmentRanges {
            ranges: vec![0..0; tracks.len()],
        })
        .collect();

    for (t_idx, track) in tracks.iter().enumerate() {
        if t_idx == anchor {
            // Exact index split on the anchor.
            for (seg, &start) in anchor_boundaries.iter().enumerate() {
                let end = if seg + 1 < n_segs {
                    anchor_boundaries[seg + 1]
                } else {
                    anchor_samples.len()
                };
                out[seg].ranges[t_idx] = start..end;
            }
            continue;
        }

        // Time-based split for non-anchor tracks.
        let scale = track.spec.timescale.max(1) as u64;
        // For each segment, find the [start_idx, end_idx) of samples whose
        // decode-start time falls in this segment's window.
        let mut seg = 0usize;
        let mut seg_start_idx = 0usize;
        let mut acc_ticks: u64 = 0;
        for (i, s) in track.samples.iter().enumerate() {
            let start_time = acc_ticks as f64 / scale as f64;
            // Advance to the segment whose window contains this sample. A sample
            // belongs to the last segment whose start_time ≤ this sample's time.
            while seg + 1 < n_segs && start_time >= start_times[seg + 1] {
                out[seg].ranges[t_idx] = seg_start_idx..i;
                seg += 1;
                seg_start_idx = i;
            }
            acc_ticks += s.duration as u64;
        }
        // Trailing samples belong to the current (last reached) segment.
        out[seg].ranges[t_idx] = seg_start_idx..track.samples.len();
    }

    out
}

// ── Streaming (incremental) TS-HLS segmentation for live input (issue #571) ─

/// One finished `.ts` media segment produced incrementally by
/// [`StreamingTsHlsSegmenter`] — the streaming analogue of one
/// [`TsHlsOutput::segments`] entry, plus the playlist metadata
/// [`StreamingTsHlsSegmenter::playlist`] needs to describe it.
#[derive(Debug, Clone)]
pub struct TsSegment {
    /// Whole-packet MPEG-2 TS bytes: PAT + PMT then the keyframe-aligned PES.
    /// Byte-identical to the corresponding segment [`TsHlsPackager::package`]
    /// would produce from the same samples in one batch call.
    pub bytes: Vec<u8>,
    /// Segment duration in seconds (the anchor track's buffered duration).
    pub duration: f64,
    /// `true` when this segment should be preceded by `#EXT-X-DISCONTINUITY`
    /// (RFC 8216 §4.3.4.3) — set via
    /// [`StreamingTsHlsSegmenter::mark_discontinuity`].
    pub discontinuous: bool,
    /// The playlist URI assigned to this segment (`"{uri_prefix}{sequence}.ts"`).
    pub uri: String,
    /// 0-based, ever-increasing sequence number of this segment. Stable for
    /// the life of the segmenter, independent of the rolling playlist window
    /// (unlike `#EXT-X-MEDIA-SEQUENCE`, which advances as segments roll off).
    pub sequence: u64,
}

/// Playlist-relevant metadata for one segment retained in the rolling window
/// — the segment bytes themselves are handed to the caller by
/// [`StreamingTsHlsSegmenter::push`]/[`finish`](StreamingTsHlsSegmenter::finish)
/// and are not kept here.
#[derive(Debug, Clone)]
struct WindowEntry {
    uri: String,
    duration: f64,
    discontinuous: bool,
    /// Absolute `#EXT-X-DISCONTINUITY-SEQUENCE` value for this segment (RFC
    /// 8216 §4.3.3.3): the cumulative count of `#EXT-X-DISCONTINUITY`
    /// boundaries crossed by, and including, this segment, counted from
    /// stream start — i.e. this segment's own `discontinuous` flag has
    /// already been folded in if `true`. Assigned once at cut time
    /// ([`StreamingTsHlsSegmenter::cut_segment`]) so the header can read it
    /// straight off whichever entry is currently the window's front, with no
    /// eviction-time bookkeeping.
    disc_seq: u64,
}

/// Per-track accumulation state for [`StreamingTsHlsSegmenter`].
struct StreamTrackState {
    spec: TrackSpec,
    /// Samples buffered so far that have not yet been flushed into a
    /// segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of `pending[0]`, in this track's media timescale ticks —
    /// the sum of the durations of every sample already flushed into an
    /// earlier segment. Mirrors `TrackState::base_decode` in
    /// [`Segmenter`](crate::segmenter::Segmenter).
    base_decode: u64,
}

/// A stateful **streaming** classic-HLS segmenter — the incremental analogue
/// of [`TsHlsPackager`] for unbounded live input, mirroring how
/// [`Segmenter`](crate::segmenter::Segmenter) drives incremental CMAF
/// fragment production.
///
/// Build it from the same [`TrackSpec`]s (in the same order) as the source
/// `Media`'s tracks, `push` coded samples in decode order, and pull finished
/// `.ts` segments from `push`'s return value; `finish` emits the final
/// partial segment. [`Self::playlist`] renders the current rolling media
/// playlist over the configured window.
///
/// ```
/// use transmux::{Sample, TrackSpec};
/// use transmux::ts_hls::StreamingTsHlsSegmenter;
/// # fn spec() -> TrackSpec { unimplemented!() }
/// # fn au(sync: bool) -> Sample { Sample::from_raw(vec![0u8; 4], 3000) }
/// # if false {
/// // 2 s target segments, keep the last 3 in the rolling playlist.
/// let mut seg = StreamingTsHlsSegmenter::new(vec![spec()], 2, 3).unwrap();
/// if let Some(s) = seg.push(1, au(true)).unwrap() { /* write s.bytes */ }
/// if let Some(s) = seg.finish().unwrap() { /* write s.bytes */ }
/// let playlist = seg.playlist(); // rolling window, #EXT-X-ENDLIST after finish
/// # }
/// ```
pub struct StreamingTsHlsSegmenter {
    tracks: Vec<StreamTrackState>,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Buffered duration of the anchor's `pending` samples (media-timescale ticks).
    anchor_pending_dur: u64,
    /// Explicit discontinuity: when `true` the *next* cut is marked
    /// discontinuous. Reset to `false` after each cut.
    pending_discontinuity: bool,
    /// Set by [`Self::finish`]; makes [`Self::playlist`] emit `#EXT-X-ENDLIST`.
    finished: bool,
    /// `#EXT-X-VERSION` written to the playlist.
    pub version: u8,
    /// URI prefix for generated `.ts` segment entries (`"{prefix}{sequence}.ts"`).
    pub uri_prefix: String,
    /// Maximum number of segments retained in the rolling playlist window.
    window: usize,
    /// The segments currently in the rolling window, oldest first.
    window_segments: VecDeque<WindowEntry>,
    /// Total number of segments ever cut (also the sequence number of the
    /// next segment).
    total_segments: u64,
    /// Running cumulative count of `#EXT-X-DISCONTINUITY` boundaries cut so
    /// far (from stream start), used to stamp each [`WindowEntry::disc_seq`]
    /// at cut time. Monotonic — never decremented, independent of window
    /// eviction (RFC 8216 §4.3.3.3: the header tracks the *current leading
    /// segment's* discontinuity count, not "discontinuities that have rolled
    /// off").
    total_discontinuities: u64,
    /// Running max `#EXT-X-TARGETDURATION` ceiling ever produced (monotonic,
    /// per RFC 8216 §4.3.3.1 — never shrinks even as segments roll off).
    target_duration: u32,
}

impl StreamingTsHlsSegmenter {
    /// Create a streaming segmenter for `tracks`, cutting segments roughly
    /// every `target_secs` seconds (clamped to at least 1) on the anchor
    /// track's keyframes, and keeping at most `window` segments in the
    /// rolling media playlist returned by [`Self::playlist`].
    ///
    /// The anchor is the first video (AVC) track, else the first track — the
    /// same rule [`TsHlsPackager`] uses. `tracks` must be given in the same
    /// order the caller will later `push` matching `track_id`s, and in the
    /// same order as the source `Media`'s tracks, so the muxed PID/PMT layout
    /// matches [`TsHlsPackager::package`] exactly (needed for the two paths
    /// to produce byte-identical segments from the same input).
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate
    /// `track_id`s, or `window` is `0`.
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

        let anchor = choose_anchor(tracks.iter().map(|t| &t.config));
        let target_ticks = target_ticks_for(target_secs, tracks[anchor].timescale);

        let tracks = tracks
            .into_iter()
            .map(|spec| StreamTrackState {
                spec,
                pending: Vec::new(),
                base_decode: 0,
            })
            .collect();

        Ok(Self {
            tracks,
            anchor,
            target_ticks,
            anchor_pending_dur: 0,
            pending_discontinuity: false,
            finished: false,
            version: DEFAULT_HLS_VERSION,
            uri_prefix: String::from("seg"),
            window,
            window_segments: VecDeque::new(),
            total_segments: 0,
            total_discontinuities: 0,
            target_duration: 0,
        })
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// Mirrors [`TsHlsPackager`]'s cut rule: when the anchor track reaches a
    /// sync sample past the target duration, the samples buffered so far are
    /// cut into a `.ts` segment (returned here) *before* this sample is
    /// buffered, so the new keyframe opens the next segment. Non-anchor
    /// tracks are split by decode time exactly as
    /// [`TsHlsPackager::package`]'s `partition_tracks` splits them: pushing
    /// every sample of a `Media` through this segmenter — interleaved so that
    /// no sample is pushed before an earlier-or-equal-decode-time sample on
    /// another track — then calling [`Self::finish`] reproduces the same
    /// segment boundaries and byte-identical segments as one
    /// [`TsHlsPackager::package`] call over the whole `Media`.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, or the
    /// underlying mux fails while cutting.
    pub fn push(&mut self, track_id: u32, sample: Sample) -> Result<Option<TsSegment>> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        let mut cut = None;
        if idx == self.anchor
            && is_cut_point(
                !self.tracks[self.anchor].pending.is_empty(),
                sample.is_sync,
                self.anchor_pending_dur,
                self.target_ticks,
            )
        {
            cut = Some(self.cut_segment(false)?);
        }

        if idx == self.anchor {
            self.anchor_pending_dur += sample.duration as u64;
        }
        self.tracks[idx].pending.push(sample);
        Ok(cut)
    }

    /// Finalize the trailing partial segment (call once at end-of-stream).
    /// `None` if nothing is buffered. Every track's remaining pending samples
    /// are flushed into this one final segment regardless of decode time —
    /// mirroring `partition_tracks`'s last segment, which runs to `+∞` and
    /// absorbs the full trailing tail of every track. Also marks the
    /// segmenter finished, so [`Self::playlist`] appends `#EXT-X-ENDLIST`.
    ///
    /// # Errors
    /// Propagates a mux failure while cutting the trailing segment.
    pub fn finish(&mut self) -> Result<Option<TsSegment>> {
        self.finished = true;
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            Ok(Some(self.cut_segment(true)?))
        } else {
            Ok(None)
        }
    }

    /// Mark the *next* segment cut as a media-timeline discontinuity
    /// (RFC 8216 §4.3.4.3) — call this when the caller detects a PID/PCR
    /// reset in the live source (e.g. an upstream PMT/PCR-PID change)
    /// between one push and the next.
    pub fn mark_discontinuity(&mut self) {
        self.pending_discontinuity = true;
    }

    /// The current rolling media playlist: at most [`window`](Self::new)
    /// segments (the most recently cut), with `#EXT-X-MEDIA-SEQUENCE`
    /// advanced past every segment that has rolled out of the window and
    /// `#EXT-X-DISCONTINUITY-SEQUENCE` set to the window's current leading
    /// (oldest still-present) segment's absolute discontinuity count (RFC
    /// 8216 §4.3.3.3) — a segment carrying `#EXT-X-DISCONTINUITY` already
    /// belongs to the incremented sequence from the moment it becomes the
    /// playlist's first segment, not one eviction cycle later when it rolls
    /// off in turn. No `#EXT-X-ENDLIST` until [`Self::finish`] has been
    /// called.
    pub fn playlist(&self) -> String {
        let segments: Vec<MediaSegment> = self
            .window_segments
            .iter()
            .map(|e| MediaSegment {
                uri: e.uri.clone(),
                duration: e.duration,
                discontinuous: e.discontinuous,
                parts: vec![],
            })
            .collect();

        let media_sequence = self.total_segments - self.window_segments.len() as u64;
        let discontinuity_sequence = self
            .window_segments
            .front()
            .map(|e| e.disc_seq)
            .unwrap_or(self.total_discontinuities);

        MediaPlaylist {
            version: self.version,
            target_duration: self.target_duration.max(1),
            media_sequence,
            discontinuity_sequence,
            segments,
            endlist: self.finished,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
        }
        .to_m3u8()
    }

    /// Cut samples into one `.ts` segment.
    ///
    /// When `final_cut` is `false` (a keyframe-triggered cut from
    /// [`Self::push`]), the anchor's whole pending buffer closes out, and a
    /// non-anchor track's samples that decode-start at or after the new
    /// segment's start time stay pending for the next segment — the
    /// incremental form of `partition_tracks`'s time-based split for
    /// non-anchor tracks. When `final_cut` is `true` (from [`Self::finish`]),
    /// every track's entire pending buffer is flushed regardless of decode
    /// time, mirroring `partition_tracks`'s last segment (which runs to `+∞`
    /// and absorbs the full trailing tail of every track).
    fn cut_segment(&mut self, final_cut: bool) -> Result<TsSegment> {
        let anchor = self.anchor;
        let anchor_scale = self.tracks[anchor].spec.timescale.max(1) as u64;
        // The new segment's start time = this track's cumulative decode time
        // so far (already-flushed ticks + the pending anchor buffer) — the
        // streaming equivalent of `partition_tracks`'s `start_times[seg + 1]`.
        let next_start_ticks = self.tracks[anchor].base_decode + self.anchor_pending_dur;
        let next_start_secs = next_start_ticks as f64 / anchor_scale as f64;

        // Per-track split point: everything pending for the anchor (and for
        // every track on the final cut); time-partitioned for non-anchor
        // tracks on a regular keyframe-triggered cut.
        let split_at: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if final_cut || i == anchor {
                    return t.pending.len();
                }
                let scale = t.spec.timescale.max(1) as u64;
                let mut acc = t.base_decode;
                for (j, s) in t.pending.iter().enumerate() {
                    let start_secs = acc as f64 / scale as f64;
                    if start_secs >= next_start_secs {
                        return j;
                    }
                    acc += s.duration as u64;
                }
                t.pending.len()
            })
            .collect();

        // Ephemeral `Track`s carrying only the spec (mux_tracks_at reads
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
        let bytes = mux_tracks_at(&mux_tracks, &sample_slices, &base_dts)?;

        let (duration, ceil_secs) = segment_duration_secs(self.anchor_pending_dur, anchor_scale);
        if ceil_secs > self.target_duration {
            self.target_duration = ceil_secs;
        }

        let discontinuous = self.pending_discontinuity;
        self.pending_discontinuity = false;
        // This segment's own discontinuity (if any) already belongs to its
        // absolute disc_seq — it is the segment carrying the
        // `#EXT-X-DISCONTINUITY` tag, so the boundary is crossed here, not
        // one eviction cycle later.
        if discontinuous {
            self.total_discontinuities += 1;
        }

        // Drop the flushed prefix of each track's pending buffer and advance
        // its base_decode past it.
        for (t, &n) in self.tracks.iter_mut().zip(&split_at) {
            let dur: u64 = t.pending[..n].iter().map(|s| s.duration as u64).sum();
            t.base_decode += dur;
            t.pending.drain(..n);
        }
        self.anchor_pending_dur = 0;

        let sequence = self.total_segments;
        let uri = format!("{}{}.ts", self.uri_prefix, sequence);
        self.window_segments.push_back(WindowEntry {
            uri: uri.clone(),
            duration,
            discontinuous,
            disc_seq: self.total_discontinuities,
        });
        self.total_segments += 1;
        while self.window_segments.len() > self.window {
            self.window_segments.pop_front();
        }

        Ok(TsSegment {
            bytes,
            duration,
            discontinuous,
            uri,
            sequence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dur: u32, sync: bool) -> Sample {
        Sample {
            data: vec![0u8; 4],
            duration: dur,
            is_sync: sync,
            composition_offset: 0,
            source_timing: None,
        }
    }

    #[test]
    fn boundaries_cut_on_keyframe_past_target() {
        // Durations 1 each, sync at 0,2,4,6; target 2 ticks.
        let s: Vec<Sample> = (0..8).map(|i| sample(1, i % 2 == 0)).collect();
        let b = anchor_segment_boundaries(&s, 2);
        // Segment 0: [0,2) (buffered reaches 2 at idx2 sync). Then [2,4), [4,6), [6,8).
        assert_eq!(b, vec![0, 2, 4, 6]);
    }

    #[test]
    fn boundaries_single_when_target_exceeds_stream() {
        let s: Vec<Sample> = (0..4).map(|i| sample(1, i == 0)).collect();
        let b = anchor_segment_boundaries(&s, 1000);
        assert_eq!(b, vec![0], "one segment when target dwarfs the stream");
    }
}
