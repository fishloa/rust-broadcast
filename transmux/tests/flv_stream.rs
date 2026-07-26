//! `StreamingFlvDemux` (#738) equivalence-to-one-shot integration tests.
//!
//! Exercises [`transmux::StreamingFlvDemux`] against the same committed real
//! fixture `fixtures/flv/av.flv` (H.264 + AAC, 320x240 — the ffmpeg RTMP
//! publish capture also used by `transmux/tests/flv.rs` and
//! `transmux/tests/rtmp.rs`), proving the incremental demuxer reproduces the
//! trusted one-shot [`transmux::FlvDemux`] exactly:
//!
//! 1. Whole-buffer equivalence: the **full per-sample sequence** (per track,
//!    in order: sample bytes, duration, sync flag, composition offset) must
//!    match the one-shot demux's, not just aggregate counts/totals — a
//!    permutation or a compensating duration error would slip past a
//!    counts-only comparison but is caught here.
//! 2. Incremental equivalence: splitting the same fixture into small
//!    (100-byte) chunks, and into single bytes, across many `feed` calls
//!    reproduces the exact same event stream as one whole-buffer `feed`.
//!
//! All three tests drive [`StreamingFlvDemux`] the same way a real caller
//! (e.g. an RTMP `RtmpSource`, #738's T11b) would: `feed` then drain with
//! `poll_event` in a loop — the uniform pull idiom shared with
//! [`transmux::StreamingTsDemux`].

use broadcast_common::Unpackage;
use bytes::Bytes;
use transmux::{CodecConfig, DemuxEvent, FlvDemux, FlvError, StreamingFlvDemux};

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");

/// Feed `input` into `demux`, then drain every event it newly queued via
/// `poll_event` (FIFO) — the drain loop a real caller uses.
fn feed_and_drain(
    demux: &mut StreamingFlvDemux,
    input: &[u8],
) -> Result<Vec<DemuxEvent>, FlvError> {
    demux.feed(input)?;
    let mut events = Vec::new();
    while let Some(ev) = demux.poll_event() {
        events.push(ev);
    }
    Ok(events)
}

/// `finish` then drain the trailing events it queues.
fn finish_and_drain(demux: &mut StreamingFlvDemux) -> Vec<DemuxEvent> {
    demux.finish();
    let mut events = Vec::new();
    while let Some(ev) = demux.poll_event() {
        events.push(ev);
    }
    events
}

fn codec_kind(c: &CodecConfig) -> &'static str {
    match c {
        CodecConfig::Avc { .. } => "avc",
        CodecConfig::Aac { .. } => "aac",
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// Full per-sample equivalence (Fix 4, #738 T11a review, Minor): every
// sample's bytes/duration/sync/composition_offset, in order, per track —
// not just aggregate counts/totals.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct FullSample {
    data: Bytes,
    /// Absolute decode/presentation time (media plane step 2c) — compared
    /// across the batch and streaming demuxers so the two must agree on the
    /// recovered timeline, not just on durations.
    dts: Option<i64>,
    pts: Option<i64>,
    duration: Option<u32>,
    is_sync: bool,
    composition_offset: i32,
}

#[derive(Debug, Clone, PartialEq)]
struct FullTrack {
    codec_kind: &'static str,
    width: u16,
    height: u16,
    samples: Vec<FullSample>,
}

fn track_dims(c: &CodecConfig) -> (u16, u16) {
    match c {
        CodecConfig::Avc { width, height, .. } => (*width, *height),
        _ => (0, 0),
    }
}

/// Run the one-shot `FlvDemux` and produce the full per-track/per-sample
/// shape, for direct comparison against the streaming demux's output.
fn one_shot_full() -> Vec<FullTrack> {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("one-shot demux av.flv");
    media
        .tracks
        .iter()
        .map(|t| {
            let (width, height) = track_dims(&t.spec.config);
            FullTrack {
                codec_kind: codec_kind(&t.spec.config),
                width,
                height,
                samples: t
                    .samples
                    .iter()
                    .map(|s| FullSample {
                        data: s.data.clone(),
                        dts: s.dts,
                        pts: s.pts,
                        duration: s.duration,
                        is_sync: s.flags.is_sync,
                        composition_offset: s.composition_offset(),
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Fold a stream of `DemuxEvent`s into the full per-track/per-sample shape,
/// in `TrackAdded` emission order.
fn full_from_events(events: &[DemuxEvent]) -> Vec<FullTrack> {
    let mut tracks: Vec<FullTrack> = Vec::new();
    let mut index_by_id = std::collections::BTreeMap::new();

    for event in events {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                let (width, height) = track_dims(&spec.config);
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(FullTrack {
                    codec_kind: codec_kind(&spec.config),
                    width,
                    height,
                    samples: Vec::new(),
                });
            }
            DemuxEvent::Sample { track_id, sample } => {
                let &i = index_by_id
                    .get(track_id)
                    .expect("Sample must follow its track's TrackAdded");
                tracks[i].samples.push(FullSample {
                    data: sample.data.clone(),
                    dts: sample.dts,
                    pts: sample.pts,
                    duration: sample.duration,
                    is_sync: sample.flags.is_sync,
                    composition_offset: sample.composition_offset(),
                });
            }
            _ => {}
        }
    }
    tracks
}

#[test]
fn streaming_whole_buffer_matches_one_shot_demux() {
    let one_shot = one_shot_full();
    // Known oracle values (also asserted by `transmux/tests/flv.rs`): 2
    // tracks, AVC 320x240 with 75 samples / 3 keyframes, AAC with 131
    // samples — pinned here too so a regression in *either* demuxer's
    // fixture handling is caught, not just a streaming/one-shot mismatch.
    assert_eq!(one_shot.len(), 2, "one-shot: 2 tracks");
    assert_eq!(one_shot[0].samples.len(), 75, "one-shot: 75 video samples");
    assert_eq!(
        one_shot[0].samples.iter().filter(|s| s.is_sync).count(),
        3,
        "one-shot: 3 video keyframes"
    );
    assert_eq!(
        one_shot[1].samples.len(),
        131,
        "one-shot: 131 audio samples"
    );

    let mut streaming = StreamingFlvDemux::new();
    let mut events = feed_and_drain(&mut streaming, FLV).expect("streaming demux av.flv");
    events.extend(finish_and_drain(&mut streaming));
    let stream_full = full_from_events(&events);

    assert_eq!(
        stream_full, one_shot,
        "StreamingFlvDemux (whole buffer, one feed call) must match FlvDemux's exact \
         per-sample sequence: same tracks (codec/dims), and every sample's bytes, \
         duration, sync flag, and composition offset, in order — not just aggregate \
         counts/totals (a permutation or compensating-duration bug would otherwise \
         slip through)"
    );
}

// ---------------------------------------------------------------------------
// Streaming self-consistency (chunk-boundary independence): aggregate
// TrackSummary is enough here since `streaming_100_byte_chunks_match_whole_buffer_feed`
// below already goes on to compare the full per-sample sequence too.
// ---------------------------------------------------------------------------

/// One track's samples collected from a `DemuxEvent` stream, keyed by
/// emission-order `TrackAdded` index (0 = first track added, 1 = second).
#[derive(Debug, Default, Clone, PartialEq)]
struct TrackSummary {
    codec_kind: &'static str,
    width: u16,
    height: u16,
    sample_count: usize,
    total_bytes: usize,
    total_duration: u64,
    keyframes: usize,
}

/// Fold a stream of `DemuxEvent`s into per-track summaries, in
/// `TrackAdded` emission order.
fn summarize(events: &[DemuxEvent]) -> Vec<TrackSummary> {
    let mut summaries: Vec<TrackSummary> = Vec::new();
    let mut index_by_id = std::collections::BTreeMap::new();

    for event in events {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                let (width, height) = track_dims(&spec.config);
                index_by_id.insert(spec.track_id, summaries.len());
                summaries.push(TrackSummary {
                    codec_kind: codec_kind(&spec.config),
                    width,
                    height,
                    sample_count: 0,
                    total_bytes: 0,
                    total_duration: 0,
                    keyframes: 0,
                });
            }
            DemuxEvent::Sample { track_id, sample } => {
                let &i = index_by_id
                    .get(track_id)
                    .expect("Sample must follow its track's TrackAdded");
                summaries[i].sample_count += 1;
                summaries[i].total_bytes += sample.data.len();
                summaries[i].total_duration += sample.duration.unwrap_or(0) as u64;
                if sample.flags.is_sync {
                    summaries[i].keyframes += 1;
                }
            }
            _ => {}
        }
    }
    summaries
}

#[test]
fn streaming_100_byte_chunks_match_whole_buffer_feed() {
    let mut whole = StreamingFlvDemux::new();
    let mut whole_events = feed_and_drain(&mut whole, FLV).expect("whole-buffer feed");
    whole_events.extend(finish_and_drain(&mut whole));
    let whole_summary = summarize(&whole_events);

    let mut chunked = StreamingFlvDemux::new();
    let mut chunked_events = Vec::new();
    for chunk in FLV.chunks(100) {
        chunked_events.extend(feed_and_drain(&mut chunked, chunk).expect("chunked feed"));
    }
    chunked_events.extend(finish_and_drain(&mut chunked));
    let chunked_summary = summarize(&chunked_events);

    assert_eq!(
        chunked_summary, whole_summary,
        "feeding the real fixture in 100-byte chunks must reproduce the exact \
         same per-track summary as one whole-buffer feed call"
    );

    // Stronger than the summary: every individual sample (bytes + duration +
    // sync flag), in order, must be identical — not just aggregate totals.
    let whole_samples: Vec<_> = whole_events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::Sample { track_id, sample } => Some((
                *track_id,
                sample.data.clone(),
                sample.duration,
                sample.flags.is_sync,
            )),
            _ => None,
        })
        .collect();
    let chunked_samples: Vec<_> = chunked_events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::Sample { track_id, sample } => Some((
                *track_id,
                sample.data.clone(),
                sample.duration,
                sample.flags.is_sync,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        chunked_samples, whole_samples,
        "every sample (bytes/duration/sync), in order, must match exactly"
    );
}

#[test]
fn streaming_byte_at_a_time_matches_whole_buffer_feed() {
    let mut whole = StreamingFlvDemux::new();
    let mut whole_events = feed_and_drain(&mut whole, FLV).expect("whole-buffer feed");
    whole_events.extend(finish_and_drain(&mut whole));
    let whole_summary = summarize(&whole_events);

    let mut byte_demux = StreamingFlvDemux::new();
    let mut byte_events = Vec::new();
    for b in FLV {
        byte_events.extend(
            feed_and_drain(&mut byte_demux, std::slice::from_ref(b)).expect("byte-at-a-time feed"),
        );
    }
    byte_events.extend(finish_and_drain(&mut byte_demux));
    let byte_summary = summarize(&byte_events);

    assert_eq!(
        byte_summary, whole_summary,
        "feeding the real fixture one byte at a time must reproduce the exact \
         same per-track summary as one whole-buffer feed call (proves partial-tag \
         buffering across arbitrarily small chunk boundaries is correct)"
    );
}
