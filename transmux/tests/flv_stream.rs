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
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
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
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
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
            DemuxEvent::Sample {
                track_id, sample, ..
            } => Some((
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
            DemuxEvent::Sample {
                track_id, sample, ..
            } => Some((
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

// ---------------------------------------------------------------------------
// r04-W13 — a re-sent sequence header updates the track
// ---------------------------------------------------------------------------

/// The fixture's own AVC sequence header (`avcC`), verbatim — a real capture's
/// decoder config, 320x240 High profile.
const FIXTURE_AVCC: &[u8] = &[
    0x01, 0x4d, 0x40, 0x0d, 0xFF, 0xE1, 0x00, 0x18, 0x67, 0x4d, 0x40, 0x0d, 0xec, 0xa0, 0xa0, 0xfd,
    0x80, 0x88, 0x00, 0x00, 0x03, 0x00, 0x08, 0x00, 0x00, 0x03, 0x01, 0x90, 0x78, 0xa1, 0x4c, 0xb0,
    0x01, 0x00, 0x05, 0x68, 0xeb, 0xe3, 0xcb, 0x20,
];

/// A second, structurally valid `avcC` with a different coded size (an SPS
/// encoding a 65 520-sample width), i.e. what a publisher sends after a
/// resolution change.
const CHANGED_AVCC: &[u8] = &[
    0x01, 0x4d, 0x40, 0x0d, 0xFF, 0xE1, 0x00, 0x09, 0x67, 0x42, 0x00, 0x1f, 0xf4, 0x00, 0x1f, 0xfe,
    0xe2, 0x00,
];

/// Append one FLV tag (header + body + `PreviousTagSize`).
fn write_tag(out: &mut Vec<u8>, tag_type: u8, timestamp: u32, body: &[u8]) {
    let start = out.len();
    out.push(tag_type);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    out.extend_from_slice(&timestamp.to_be_bytes()[1..4]); // Timestamp UI24
    out.push((timestamp >> 24) as u8); // TimestampExtended
    out.extend_from_slice(&[0, 0, 0]); // StreamID
    out.extend_from_slice(body);
    let size = (out.len() - start) as u32;
    out.extend_from_slice(&size.to_be_bytes());
}

/// An AVC sequence-header tag body carrying `avcc`.
fn avc_seq_body(avcc: &[u8]) -> Vec<u8> {
    let mut body = vec![0x17, 0x00, 0x00, 0x00, 0x00];
    body.extend_from_slice(avcc);
    body
}

/// An AVC NALU tag body with one length-prefixed NAL.
fn avc_nalu_body(payload: u8) -> Vec<u8> {
    let mut body = vec![0x17, 0x01, 0x00, 0x00, 0x00];
    body.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x41, payload]);
    body
}

/// An FLV with one sequence header, one NALU, then a **second** sequence
/// header whose SPS describes a different coded size, then another NALU —
/// a resolution change mid-publish (`OBS`/`ffmpeg` do this routinely).
fn flv_with_config_change() -> Vec<u8> {
    let mut out = FLV[..13].to_vec(); // real header + PreviousTagSize0
    write_tag(&mut out, 9, 0, &avc_seq_body(FIXTURE_AVCC));
    write_tag(&mut out, 9, 0, &avc_nalu_body(0xAA));
    write_tag(&mut out, 9, 40, &avc_seq_body(CHANGED_AVCC));
    write_tag(&mut out, 9, 40, &avc_nalu_body(0xBB));
    out
}

/// The second sequence header must surface as a config change on the same
/// track. Before the fix the `SEQUENCE_HEADER` arm was guarded by
/// `track_id.is_none()`, so it was skipped entirely: the stream kept reporting
/// the stale 320x240 avcC and every downstream init segment described the old
/// stream.
#[test]
fn resent_sequence_header_updates_the_track() {
    let mut demux = StreamingFlvDemux::new();
    let events = feed_and_drain(&mut demux, &flv_with_config_change()).expect("feed");

    let mut added = 0usize;
    let mut updated: Vec<(u32, u16, u16)> = Vec::new();
    for ev in &events {
        match ev {
            DemuxEvent::TrackAdded(spec) => {
                added += 1;
                let CodecConfig::Avc { width, height, .. } = &spec.config else {
                    panic!("video track must be AVC");
                };
                assert_eq!((*width, *height), (320, 240), "first config");
            }
            DemuxEvent::TrackUpdated(spec) => {
                let CodecConfig::Avc { width, height, .. } = &spec.config else {
                    panic!("video track must be AVC");
                };
                updated.push((spec.track_id, *width, *height));
            }
            _ => {}
        }
    }
    assert_eq!(
        added, 1,
        "exactly one TrackAdded — the track keeps its identity"
    );
    assert_eq!(
        updated,
        vec![(1, 65_520, 48)],
        "the re-sent sequence header reports the new coded size on the same track"
    );
}

/// A re-sent **AAC** sequence header (sample rate / channel count change)
/// updates the audio track the same way.
#[test]
fn resent_aac_sequence_header_updates_the_track() {
    let mut out = FLV[..13].to_vec();
    // Stereo 44100 (ASC 0x12 0x10), a sample, then mono 48000 (0x11 0x88).
    write_tag(&mut out, 8, 0, &[0xAF, 0x00, 0x12, 0x10]);
    write_tag(&mut out, 8, 0, &[0xAF, 0x01, 0x00, 0x11, 0x22]);
    write_tag(&mut out, 8, 23, &[0xAF, 0x00, 0x11, 0x88]);
    write_tag(&mut out, 8, 23, &[0xAF, 0x01, 0x00, 0x33, 0x44]);

    let mut demux = StreamingFlvDemux::new();
    let events = feed_and_drain(&mut demux, &out).expect("feed");
    let updated: Vec<(u32, u16, u32)> = events
        .iter()
        .filter_map(|ev| match ev {
            DemuxEvent::TrackUpdated(spec) => {
                let CodecConfig::Aac {
                    sample_rate,
                    channel_count,
                    ..
                } = &spec.config
                else {
                    panic!("audio track must be AAC");
                };
                Some((spec.track_id, *channel_count, *sample_rate))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        updated,
        vec![(1, 1, 48_000)],
        "the re-sent ASC reports mono 48000 on the same track"
    );
}

// ---------------------------------------------------------------------------
// r04-W13(b) — a corrupt tag does not poison the ingest forever
// ---------------------------------------------------------------------------

/// A sequence header whose `avcC` is structurally corrupt makes `feed` return
/// `Err`. That tag must be consumed: before the fix it stayed in `pending`, so
/// every later `feed` re-parsed the same bytes and returned the same error,
/// and the live ingest could never continue past one bad tag.
#[test]
fn corrupt_tag_is_drained_and_the_stream_continues() {
    let mut out = FLV[..13].to_vec();
    // An avcC that declares one SPS whose length runs past the tag body —
    // `AVCDecoderConfigurationRecord::parse` rejects it.
    write_tag(
        &mut out,
        9,
        0,
        &avc_seq_body(&[0x01, 0x4d, 0x40, 0x0d, 0xFF, 0xE1, 0x00, 0xFF]),
    );
    // A perfectly good sequence header + NALU follow.
    write_tag(&mut out, 9, 0, &avc_seq_body(FIXTURE_AVCC));
    write_tag(&mut out, 9, 0, &avc_nalu_body(0xAA));

    let mut demux = StreamingFlvDemux::new();
    let err = demux
        .feed(&out)
        .expect_err("the corrupt avcC must be reported");
    assert!(
        matches!(err, FlvError::Codec(_)),
        "expected a codec-config error, got {err:?}"
    );

    // Bites: before the fix the same corrupt tag was re-parsed here and this
    // returned the identical error again (and forever after).
    let events = feed_and_drain(&mut demux, &[]).expect("the next feed must make progress");
    let resumed: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            DemuxEvent::TrackAdded(spec) => Some(spec.track_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        resumed,
        vec![1],
        "the good sequence header after the corrupt tag resolves the track"
    );
}

// ---------------------------------------------------------------------------
// Item 1 — an unchanged re-sent sequence header emits nothing
// ---------------------------------------------------------------------------

/// Encoders that repeat their sequence header on every keyframe must not make
/// the demuxer emit `TrackUpdated` each time: a consumer treats that event as
/// "rebuild the init segment", so a per-keyframe resend would churn it hundreds
/// of times a minute for a config that never changed.
#[test]
fn identical_resent_sequence_header_emits_no_event() {
    let mut out = FLV[..13].to_vec();
    // The same real avcC three times, interleaved with NALUs.
    for (i, ts) in [0u32, 40, 80].into_iter().enumerate() {
        write_tag(&mut out, 9, ts, &avc_seq_body(FIXTURE_AVCC));
        write_tag(&mut out, 9, ts, &avc_nalu_body(i as u8));
    }

    let mut demux = StreamingFlvDemux::new();
    let events = feed_and_drain(&mut demux, &out).expect("feed");

    let added = events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    let updated = events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackUpdated(_)))
        .count();
    assert_eq!(added, 1, "one track");
    // Bites: every repeat emitted a TrackUpdated before the dedupe.
    assert_eq!(updated, 0, "an identical re-sent avcC emits nothing");
}

/// The same rule for AAC, and the audio track must equally stay silent.
#[test]
fn identical_resent_aac_sequence_header_emits_no_event() {
    let mut out = FLV[..13].to_vec();
    for ts in [0u32, 23, 46] {
        write_tag(&mut out, 8, ts, &[0xAF, 0x00, 0x12, 0x10]);
        write_tag(&mut out, 8, ts, &[0xAF, 0x01, 0x00, 0x11, 0x22]);
    }

    let mut demux = StreamingFlvDemux::new();
    let events = feed_and_drain(&mut demux, &out).expect("feed");
    let updated = events
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackUpdated(_)))
        .count();
    assert_eq!(updated, 0, "an identical re-sent ASC emits nothing");
}

/// A *changed* header still reports, and a change back reports again — the
/// dedupe compares bytes, it does not latch "already updated once".
#[test]
fn changed_sequence_header_still_emits_track_updated() {
    let mut out = FLV[..13].to_vec();
    for (avcc, ts) in [
        (FIXTURE_AVCC, 0u32),
        (FIXTURE_AVCC, 40),  // identical — silent
        (CHANGED_AVCC, 80),  // changed
        (CHANGED_AVCC, 120), // identical again — silent
        (FIXTURE_AVCC, 160), // changed back
    ] {
        write_tag(&mut out, 9, ts, &avc_seq_body(avcc));
        write_tag(&mut out, 9, ts, &avc_nalu_body(0x55));
    }

    let mut demux = StreamingFlvDemux::new();
    let events = feed_and_drain(&mut demux, &out).expect("feed");
    let sizes: Vec<(u16, u16)> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::TrackUpdated(spec) => match &spec.config {
                CodecConfig::Avc { width, height, .. } => Some((*width, *height)),
                other => panic!("video track must be AVC, got {other:?}"),
            },
            _ => None,
        })
        .collect();
    // Bites: without the dedupe this is 4 entries, all identical in pairs.
    assert_eq!(
        sizes,
        vec![(65_520, 48), (320, 240)],
        "exactly the two real changes are reported"
    );
}

// ---------------------------------------------------------------------------
// Item 3 — an undetermined AAC channel count is not fabricated
// ---------------------------------------------------------------------------

/// `channelConfiguration == 0` means a `program_config_element` in the data
/// stream carries the mapping; it is *not* "0 channels". Reserved values
/// (8..=15) are equally undetermined. Both must report the documented
/// "unknown" placeholder rather than a made-up count.
#[test]
fn undetermined_aac_channel_count_is_unknown_not_zero_fabricated() {
    // AOT 5 bits, sfi 4 bits, channelConfiguration 4 bits: byte0 = 0x11 is
    // AAC-LC (2) at sfi 3 (48 kHz); byte1's low nibble<<3 is the config, which
    // matches the real 7.1 fixture's `11 B8` (0xB8 >> 3 & 0xF = 7).
    let cases: [([u8; 2], u16); 3] = [
        ([0x11, 0x80], 0), // channelConfiguration 0: in-band PCE -> unknown
        ([0x11, 0xB8], 8), // channelConfiguration 7: 7.1 -> 8 channels
        ([0x11, 0xC0], 0), // channelConfiguration 8: reserved -> unknown
    ];
    for (asc, expected) in cases {
        let mut out = FLV[..13].to_vec();
        write_tag(&mut out, 8, 0, &[0xAF, 0x00, asc[0], asc[1]]);
        write_tag(&mut out, 8, 0, &[0xAF, 0x01, 0x00, 0xAA, 0xBB]);

        let mut demux = FlvDemux::new();
        let media = demux.unpackage(&out).expect("demux");
        let CodecConfig::Aac { channel_count, .. } = media.tracks[0].config() else {
            panic!("track 0 must be AAC");
        };
        assert_eq!(*channel_count, expected, "ASC {asc:02X?}");
    }
}

/// The streaming demuxer must report the same value for the same ASCs.
#[test]
fn streaming_undetermined_aac_channel_count_matches() {
    for (asc, expected) in [
        ([0x11u8, 0x80u8], 0u16),
        ([0x11, 0xB8], 8),
        ([0x11, 0xC0], 0),
    ] {
        let mut out = FLV[..13].to_vec();
        write_tag(&mut out, 8, 0, &[0xAF, 0x00, asc[0], asc[1]]);
        write_tag(&mut out, 8, 0, &[0xAF, 0x01, 0x00, 0xAA, 0xBB]);

        let mut demux = StreamingFlvDemux::new();
        let events = feed_and_drain(&mut demux, &out).expect("feed");
        let got = events.iter().find_map(|e| match e {
            DemuxEvent::TrackAdded(spec) => match &spec.config {
                CodecConfig::Aac { channel_count, .. } => Some(*channel_count),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(got, Some(expected), "ASC {asc:02X?}");
    }
}
