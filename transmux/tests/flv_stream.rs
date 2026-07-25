//! `StreamingFlvDemux` (#738) equivalence-to-one-shot integration tests.
//!
//! Exercises [`transmux::StreamingFlvDemux`] against the same committed real
//! fixture `fixtures/flv/av.flv` (H.264 + AAC, 320x240 — the ffmpeg RTMP
//! publish capture also used by `transmux/tests/flv.rs` and
//! `transmux/tests/rtmp.rs`), proving the incremental demuxer reproduces the
//! trusted one-shot [`transmux::FlvDemux`] exactly:
//!
//! 1. Whole-buffer equivalence: same tracks (codec + dims), same total
//!    sample count/bytes/duration per track as the one-shot demux.
//! 2. Incremental equivalence: splitting the same fixture into small
//!    (100-byte) chunks, and into single bytes, across many `feed` calls
//!    reproduces the exact same event stream as one whole-buffer `feed`.

use broadcast_common::Unpackage;
use transmux::{DemuxEvent, FlvDemux, StreamingFlvDemux};

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");

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

fn codec_kind(c: &transmux::CodecConfig) -> &'static str {
    match c {
        transmux::CodecConfig::Avc { .. } => "avc",
        transmux::CodecConfig::Aac { .. } => "aac",
        _ => "other",
    }
}

/// Fold a stream of `DemuxEvent`s into per-track summaries, in
/// `TrackAdded` emission order.
fn summarize(events: &[DemuxEvent]) -> Vec<TrackSummary> {
    let mut summaries: Vec<TrackSummary> = Vec::new();
    let mut index_by_id = std::collections::BTreeMap::new();

    for event in events {
        match event {
            DemuxEvent::TrackAdded(track) => {
                let (width, height) = match &track.spec.config {
                    transmux::CodecConfig::Avc { width, height, .. } => (*width, *height),
                    _ => (0, 0),
                };
                index_by_id.insert(track.spec.track_id, summaries.len());
                summaries.push(TrackSummary {
                    codec_kind: codec_kind(&track.spec.config),
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
                summaries[i].total_duration += sample.duration as u64;
                if sample.is_sync {
                    summaries[i].keyframes += 1;
                }
            }
            _ => {}
        }
    }
    summaries
}

/// Run the one-shot `FlvDemux` and produce the same `TrackSummary` shape,
/// for direct comparison against the streaming demux's output.
fn one_shot_summary() -> Vec<TrackSummary> {
    let mut demux = FlvDemux::new();
    let media = demux.unpackage(FLV).expect("one-shot demux av.flv");
    media
        .tracks
        .iter()
        .map(|t| {
            let (width, height) = match &t.spec.config {
                transmux::CodecConfig::Avc { width, height, .. } => (*width, *height),
                _ => (0, 0),
            };
            TrackSummary {
                codec_kind: codec_kind(&t.spec.config),
                width,
                height,
                sample_count: t.samples.len(),
                total_bytes: t.samples.iter().map(|s| s.data.len()).sum(),
                total_duration: t.samples.iter().map(|s| s.duration as u64).sum(),
                keyframes: t.samples.iter().filter(|s| s.is_sync).count(),
            }
        })
        .collect()
}

#[test]
fn streaming_whole_buffer_matches_one_shot_demux() {
    let one_shot = one_shot_summary();
    // Known oracle values (also asserted by `transmux/tests/flv.rs`): 2
    // tracks, AVC 320x240 with 75 samples / 3 keyframes, AAC with 131
    // samples — pinned here too so a regression in *either* demuxer's
    // fixture handling is caught, not just a streaming/one-shot mismatch.
    assert_eq!(one_shot.len(), 2, "one-shot: 2 tracks");
    assert_eq!(one_shot[0].sample_count, 75, "one-shot: 75 video samples");
    assert_eq!(one_shot[0].keyframes, 3, "one-shot: 3 video keyframes");
    assert_eq!(one_shot[1].sample_count, 131, "one-shot: 131 audio samples");

    let mut streaming = StreamingFlvDemux::new();
    let mut events = streaming.feed(FLV).expect("streaming demux av.flv");
    events.extend(streaming.finish());
    let stream_summary = summarize(&events);

    assert_eq!(
        stream_summary, one_shot,
        "StreamingFlvDemux (whole buffer, one feed call) must match FlvDemux exactly: \
         same tracks, same per-track sample count/bytes/duration/keyframes"
    );
}

#[test]
fn streaming_100_byte_chunks_match_whole_buffer_feed() {
    let mut whole = StreamingFlvDemux::new();
    let mut whole_events = whole.feed(FLV).expect("whole-buffer feed");
    whole_events.extend(whole.finish());
    let whole_summary = summarize(&whole_events);

    let mut chunked = StreamingFlvDemux::new();
    let mut chunked_events = Vec::new();
    for chunk in FLV.chunks(100) {
        chunked_events.extend(chunked.feed(chunk).expect("chunked feed"));
    }
    chunked_events.extend(chunked.finish());
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
                sample.is_sync,
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
                sample.is_sync,
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
    let mut whole_events = whole.feed(FLV).expect("whole-buffer feed");
    whole_events.extend(whole.finish());
    let whole_summary = summarize(&whole_events);

    let mut byte_demux = StreamingFlvDemux::new();
    let mut byte_events = Vec::new();
    for b in FLV {
        byte_events.extend(
            byte_demux
                .feed(std::slice::from_ref(b))
                .expect("byte-at-a-time feed"),
        );
    }
    byte_events.extend(byte_demux.finish());
    let byte_summary = summarize(&byte_events);

    assert_eq!(
        byte_summary, whole_summary,
        "feeding the real fixture one byte at a time must reproduce the exact \
         same per-track summary as one whole-buffer feed call (proves partial-tag \
         buffering across arbitrarily small chunk boundaries is correct)"
    );
}
