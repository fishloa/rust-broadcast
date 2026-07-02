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
//! (RFC 8216 §4.3.3). `#EXT-X-DISCONTINUITY` is **not** emitted: this packager
//! produces a single continuous timeline from one contiguous IR, so no
//! discontinuity marker is warranted.
//!
//! # Spec
//!
//! - **HLS media playlist**: RFC 8216 §4.1 / §4.3.2 (`#EXTINF`) / §4.3.3
//!   (`#EXT-X-TARGETDURATION`, `#EXT-X-MEDIA-SEQUENCE`, `#EXT-X-ENDLIST`).
//! - **TS segment constraints (PAT/PMT at each segment start)**:
//!   ISO/IEC 13818-1 §2.4.4 — PSI is periodically repeated so any access point
//!   is self-describing.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::Package;

use crate::error::{Error, Result};
use crate::hls::{MediaPlaylist, MediaSegment};
use crate::media::{Media, Track};
use crate::pipeline::{CodecConfig, Sample};
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
        let anchor = media
            .tracks
            .iter()
            .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .unwrap_or(0);

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
            let duration = anchor_ticks as f64 / ts_scale as f64;
            // #EXT-X-TARGETDURATION is an integer ≥ every #EXTINF (RFC 8216
            // §4.3.3.1: the rounded max segment duration).
            let ceil_secs = anchor_ticks.div_ceil(ts_scale) as u32;
            if ceil_secs > target_duration {
                target_duration = ceil_secs;
            }
            playlist_segments.push(MediaSegment {
                uri: format!("{}{}.ts", self.uri_prefix, i),
                duration,
            });
        }

        let playlist = MediaPlaylist {
            version: self.version,
            target_duration: target_duration.max(1),
            media_sequence: self.media_sequence,
            segments: playlist_segments,
            endlist: true,
            extra_tags: vec![],
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
        let ts_scale = anchor.spec.timescale.max(1) as u64;
        (self.target_secs as u64 * ts_scale).max(1)
    }
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
        if i > 0 && s.is_sync && buffered >= target_ticks {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dur: u32, sync: bool) -> Sample {
        Sample {
            data: vec![0u8; 4],
            duration: dur,
            is_sync: sync,
            composition_offset: 0,
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
