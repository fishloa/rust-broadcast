//! `StreamingTsHlsSegmenter` gate — incremental/live classic-HLS segmentation
//! (issue #571), verified against the existing batch `TsHlsPackager` on the
//! same real fixture (`fixtures/ts/h264-aac.ts`, demuxed via `TsDemux`).
//!
//! Pipeline: `ir = TsDemux(h264-aac.ts)` → push every sample of `ir` through
//! `StreamingTsHlsSegmenter` in global decode-time order → compare the
//! resulting `.ts` segments against `TsHlsPackager::package(&ir)`'s batch
//! output. Byte-identical segments prove the streaming cut/partition logic
//! reproduces the batch algorithm exactly — this cannot be faked by a
//! raw-passthrough implementation, since it requires correct keyframe-aligned
//! cutting *and* correct non-anchor (audio) time-partitioning at each cut.

use crate::hub_fixtures;
use crate::hub_fixtures::dummy_avc_config;
use broadcast_common::Package;
use broadcast_common::Unpackage;
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::Timescale;
use transmux::TrackId;
use transmux::media::Media;
use transmux::segmenter::MAX_PENDING_SAMPLES_PER_TRACK;
use transmux::{
    CodecConfig, Sample, StreamingTsHlsSegmenter, TrackSpec, TsDemux, TsHlsPackager, TsSegment,
};

// ── Fixture + pipeline ───────────────────────────────────────────────────────

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn load_ts() -> Vec<u8> {
    let path = test_fixtures::path("ts/h264-aac.ts");
    std::fs::read(path).expect("h264-aac.ts fixture must exist")
}

fn demux(ts: &[u8]) -> Media {
    TsDemux::new().unpackage(ts).expect("demux TS")
}

fn track_specs(ir: &Media) -> Vec<TrackSpec> {
    ir.tracks.iter().map(|t| t.spec.clone()).collect()
}

/// Every `(track_position, sample_index)` pair in the IR, in **global decode-time
/// order** — the realistic live-ingest push order (video/audio interleaved by
/// wall-clock arrival, not "all of one track then all of the other"). Computed
/// independently of `StreamingTsHlsSegmenter`/`TsHlsPackager` internals: purely
/// from each track's own timescale and per-sample durations.
fn merged_decode_order(ir: &Media) -> Vec<(usize, usize)> {
    let mut items: Vec<(f64, usize, usize)> = Vec::new();
    for (tpos, t) in ir.tracks.iter().enumerate() {
        let scale = t.spec.timescale.get() as u64;
        let mut acc: u64 = 0;
        for (i, s) in t.samples.iter().enumerate() {
            let start_secs = acc as f64 / scale as f64;
            items.push((start_secs, tpos, i));
            acc += s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64;
        }
    }
    // Stable sort by decode-start time; ties keep original (track, sample)
    // order so within-track ordering is never disturbed.
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
    items.into_iter().map(|(_, tpos, i)| (tpos, i)).collect()
}

/// Feed every sample of `ir` through `seg` one at a time, in global
/// decode-time order, returning every segment emitted by `push` (in order),
/// followed by `finish`'s trailing segment (if any).
fn feed_all(ir: &Media, seg: &mut StreamingTsHlsSegmenter) -> Vec<TsSegment> {
    for (tpos, i) in merged_decode_order(ir) {
        let track_id = ir.tracks[tpos].spec.track_id;
        let sample = ir.tracks[tpos].samples[i].clone();
        seg.push(track_id, sample).expect("push");
    }
    seg.finish().expect("finish");
    seg.take_ready()
}

fn extract_tag_u64(playlist: &str, tag: &str) -> u64 {
    playlist
        .lines()
        .find_map(|l| l.strip_prefix(tag))
        .unwrap_or_else(|| panic!("playlist missing {tag}"))
        .trim()
        .parse()
        .unwrap()
}

/// URIs of every `#EXTINF`-declared segment, in playlist order.
fn extract_uris(playlist: &str) -> Vec<String> {
    let mut uris = Vec::new();
    let mut lines = playlist.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("#EXTINF:") {
            let uri = lines.next().expect("URI after #EXTINF");
            uris.push(uri.to_string());
        }
    }
    uris
}

// ── Test 1 — batch-equivalence ────────────────────────────────────────────────

#[test]
fn streaming_reproduces_batch_segment_boundaries_and_bytes() {
    let ir = demux(&load_ts());
    let batch = TsHlsPackager::new(1).package(&ir).expect("batch package");

    let mut seg = StreamingTsHlsSegmenter::new(track_specs(&ir), 1, usize::MAX)
        .expect("construct streaming segmenter");
    let streamed = feed_all(&ir, &mut seg);

    assert_eq!(
        streamed.len(),
        batch.segments.len(),
        "streaming must cut the same number of segments as the batch packager"
    );
    for (i, (s, b)) in streamed.iter().zip(&batch.segments).enumerate() {
        assert_eq!(
            &s.bytes, b,
            "segment {i}: streaming bytes must be byte-identical to the batch segment"
        );
    }
}

// ── Test 2 — incremental / bounded: progress before finish, no data lost ─────

#[test]
fn streaming_emits_segments_progressively_and_loses_nothing() {
    let ir = demux(&load_ts());
    let batch = TsHlsPackager::new(1).package(&ir).expect("batch package");

    let mut seg = StreamingTsHlsSegmenter::new(track_specs(&ir), 1, usize::MAX)
        .expect("construct streaming segmenter");

    let order = merged_decode_order(&ir);
    assert!(
        order.len() > 1,
        "fixture must carry more than one sample to prove incremental behaviour"
    );

    for (tpos, i) in &order {
        let track_id = ir.tracks[*tpos].spec.track_id;
        let sample = ir.tracks[*tpos].samples[*i].clone();
        seg.push(track_id, sample).expect("push");
    }
    let emitted_before_finish: Vec<TsSegment> = seg.take_ready();
    assert!(
        !emitted_before_finish.is_empty(),
        "at least one segment must be available before finish() (progressive emission)"
    );
    assert!(
        emitted_before_finish.len() < batch.segments.len(),
        "not every segment can have been flushed before finish() cuts the trailing partial"
    );

    let mut all: Vec<TsSegment> = emitted_before_finish;
    seg.finish().expect("finish");
    all.extend(seg.take_ready());

    // No sample lost or duplicated: concatenation matches the batch output
    // exactly (segment-for-segment, matching test 1's stronger claim).
    assert_eq!(all.len(), batch.segments.len());
    let concat_stream: Vec<u8> = all.iter().flat_map(|s| s.bytes.clone()).collect();
    let concat_batch: Vec<u8> = batch.segments.iter().flatten().copied().collect();
    assert_eq!(
        concat_stream, concat_batch,
        "concatenation of incrementally-pushed segments must equal the batch output"
    );
}

// ── Test 3 — rolling playlist: window + media-sequence + discontinuity ───────

#[test]
fn rolling_playlist_windows_and_advances_media_sequence() {
    let ir = demux(&load_ts());
    let window = 1usize;

    let mut seg =
        StreamingTsHlsSegmenter::new(track_specs(&ir), 1, window).expect("construct segmenter");

    for (tpos, i) in merged_decode_order(&ir) {
        let track_id = ir.tracks[tpos].spec.track_id;
        let sample = ir.tracks[tpos].samples[i].clone();
        seg.push(track_id, sample).expect("push");
    }
    let emitted: Vec<TsSegment> = seg.take_ready();
    let m = emitted.len();
    assert!(
        m > window,
        "fixture at 1s target must cut more segments ({m}) than the window ({window})"
    );

    // Still live: no #EXT-X-ENDLIST, window holds exactly the last N segments,
    // and #EXT-X-MEDIA-SEQUENCE has advanced past the rolled-off segments.
    let pl = seg.playlist().unwrap();
    assert!(
        !pl.contains("#EXT-X-ENDLIST"),
        "no ENDLIST while live (before finish())"
    );
    let media_sequence = extract_tag_u64(&pl, "#EXT-X-MEDIA-SEQUENCE:");
    assert_eq!(
        media_sequence,
        (m - window) as u64,
        "media sequence must advance to M - N"
    );
    let uris = extract_uris(&pl);
    assert_eq!(uris.len(), window, "playlist must list exactly N segments");
    let expected_uris: Vec<String> = emitted[m - window..]
        .iter()
        .map(|s| s.uri.clone())
        .collect();
    assert_eq!(
        uris, expected_uris,
        "playlist must list exactly the last N segments, in order"
    );

    // finish() flushes the trailing partial segment and appends ENDLIST.
    seg.finish().expect("finish");
    let last = seg.take_ready().pop();
    let pl2 = seg.playlist().unwrap();
    assert!(
        pl2.trim_end().ends_with("#EXT-X-ENDLIST"),
        "ENDLIST must appear once finished"
    );
    if let Some(tail) = last {
        // The just-finished trailing segment is now the newest in the window.
        let uris2 = extract_uris(&pl2);
        assert_eq!(uris2.last().unwrap(), &tail.uri);
    }
}

// ── Test 4 — #EXT-X-DISCONTINUITY-SEQUENCE reflects the window's leading
//    segment, not evictions (issue #629) ───────────────────────────────────
//
// RFC 8216 §4.3.3.3: the header carries the discontinuity-sequence number of
// the FIRST segment currently in the playlist. A segment carrying
// `#EXT-X-DISCONTINUITY` already belongs to the incremented sequence from the
// moment it becomes the window's leading (oldest) segment — not one eviction
// cycle later, once *it* rolls off in turn.

fn single_video_track() -> TrackSpec {
    TrackSpec::new(
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config: dummy_avc_config(),
            width: 320,
            height: 240,
        },
    )
}

/// One 4-byte-length-prefixed H.264 NAL sample: type 5 (IDR) for sync
/// samples, type 1 (non-IDR slice) otherwise — matches
/// `AVCDecoderConfigurationRecord::length_size_minus_one == 3` above.
fn nal_sample(is_sync: bool, dur: u32) -> Sample {
    let nal_type: u8 = if is_sync { 5 } else { 1 };
    let payload = [nal_type, 0xAA, 0xBB];
    let mut data = Vec::with_capacity(4 + payload.len());
    data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    data.extend_from_slice(&payload);
    // Duration-driven segmenter: no absolute source time for these synthetic
    // samples (media plane step 2c — never fabricated).
    Sample::new(data, None, None, Some(dur), is_sync).expect("valid timing")
}

/// Build the worked example from issue #629: 5 segments s0..s4 (target 1s @
/// 90_000 ticks/s, so a cut fires once 90_000 ticks are buffered and the next
/// sample is a sync sample), with s2 explicitly marked discontinuous via
/// `mark_discontinuity()` before it is cut. Returns the segmenter plus the
/// playlist snapshot captured right after each of the 5 cuts.
fn build_worked_example(window: usize) -> (StreamingTsHlsSegmenter, Vec<String>) {
    let mut seg = StreamingTsHlsSegmenter::new(vec![single_video_track()], 1, window)
        .expect("construct segmenter");
    let mut snapshots = Vec::new();

    // sync -> opens s0 (45_000 ticks buffered)
    seg.push(TrackId::new(1), nal_sample(true, 45_000)).unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "opening sync sample must not cut"
    );
    // non-sync -> 90_000 ticks buffered (>= target)
    seg.push(TrackId::new(1), nal_sample(false, 45_000))
        .unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "buffering non-sync sample must not cut"
    );

    // sync -> cuts s0 (continuous); opens s1
    seg.push(TrackId::new(1), nal_sample(true, 45_000)).unwrap();
    assert_eq!(seg.take_ready().len(), 1, "sync sample must cut s0");
    seg.push(TrackId::new(1), nal_sample(false, 45_000))
        .unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "buffering non-sync sample must not cut"
    );

    // sync -> cuts s1 (continuous); opens s2
    seg.push(TrackId::new(1), nal_sample(true, 45_000)).unwrap();
    assert_eq!(seg.take_ready().len(), 1, "sync sample must cut s1");
    snapshots.push(seg.playlist().unwrap());
    // Mark s2 (currently buffering) as the next segment to be cut discontinuous.
    seg.mark_discontinuity();
    seg.push(TrackId::new(1), nal_sample(false, 45_000))
        .unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "buffering non-sync sample must not cut"
    );

    // sync -> cuts s2 (discontinuous); opens s3
    seg.push(TrackId::new(1), nal_sample(true, 45_000)).unwrap();
    let s2 = seg.take_ready();
    assert_eq!(s2.len(), 1, "sync sample must cut s2 singly");
    assert!(s2[0].discontinuous, "s2 must be discontinuous");
    snapshots.push(seg.playlist().unwrap());
    seg.push(TrackId::new(1), nal_sample(false, 45_000))
        .unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "buffering non-sync sample must not cut"
    );

    // sync -> cuts s3 (continuous); opens s4
    seg.push(TrackId::new(1), nal_sample(true, 45_000)).unwrap();
    assert_eq!(seg.take_ready().len(), 1, "sync sample must cut s3");
    snapshots.push(seg.playlist().unwrap());
    seg.push(TrackId::new(1), nal_sample(false, 45_000))
        .unwrap();
    assert!(
        seg.take_ready().is_empty(),
        "buffering non-sync sample must not cut"
    );

    // finish() -> cuts s4 (continuous, final flush)
    seg.finish().unwrap();
    assert_eq!(seg.take_ready().len(), 1, "finish must cut s4");
    snapshots.push(seg.playlist().unwrap());

    (seg, snapshots)
}

/// Compute each segment's true **Discontinuity Sequence Number** from a raw
/// playlist, per RFC 8216 §6.2.1's literal definition: "the value of the
/// EXT-X-DISCONTINUITY-SEQUENCE tag (or zero if none) plus the number of
/// EXT-X-DISCONTINUITY tags in the Playlist preceding the URI line of the
/// segment." Returns `(uri, dsn)` pairs in playlist order. This is the
/// client-observable value a real HLS player computes — asserting against
/// it (rather than the raw header integer alone) is what actually validates
/// spec compliance; a header value can be "correct-looking" in isolation
/// while still yielding a wrong DSN once combined with the inline tags still
/// visible in the window (exactly how issue #629's original "fix" slipped
/// through: it changed the header without accounting for the inline tag on
/// the same leading segment, double-counting that segment's own boundary).
fn client_computed_dsns(playlist: &str) -> Vec<(String, u64)> {
    let base = playlist
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:"))
        .map(|v| v.trim().parse::<u64>().unwrap())
        .unwrap_or(0);

    let mut out = Vec::new();
    let mut tags_seen = 0u64;
    let mut lines = playlist.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("#EXT-X-DISCONTINUITY")
            && !line.starts_with("#EXT-X-DISCONTINUITY-SEQUENCE")
        {
            tags_seen += 1;
        }
        if line.starts_with("#EXTINF:") {
            let uri = lines.next().expect("URI after #EXTINF");
            out.push((uri.to_string(), base + tags_seen));
        }
    }
    out
}

#[test]
fn discontinuity_sequence_is_spec_correct_and_stable_across_window_rollover() {
    let (_seg, snapshots) = build_worked_example(2);
    assert_eq!(
        snapshots.len(),
        4,
        "expected one playlist snapshot after each of the last 4 cuts"
    );

    // The TRUE discontinuity sequence number for each segment, independent
    // of window state: s0=0, s1=0, s2=1 (s2 itself is discontinuous), s3=1,
    // s4=1 — the cumulative count of discontinuity boundaries crossed by,
    // and including, that segment, from stream start.
    let true_dsn = |uri: &str| -> u64 {
        match uri {
            "seg0.ts" | "seg1.ts" => 0,
            "seg2.ts" | "seg3.ts" | "seg4.ts" => 1,
            other => panic!("unexpected uri {other}"),
        }
    };

    let mut seen_dsn: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for (i, snapshot) in snapshots.iter().enumerate() {
        for (uri, dsn) in client_computed_dsns(snapshot) {
            assert_eq!(
                dsn,
                true_dsn(&uri),
                "snapshot {i}: client-computed DSN for {uri} must equal its true \
                 cumulative discontinuity count (RFC 8216 §6.2.1: header + \
                 preceding #EXT-X-DISCONTINUITY tags in THIS playlist)"
            );
            // A segment's DSN must never change across reloads while it
            // remains in the playlist (RFC 8216 §6.2.2 requires
            // EXT-X-DISCONTINUITY-SEQUENCE to only ever increase, and the
            // whole point of the tag is a stable, monotonic numbering).
            if let Some(&prev) = seen_dsn.get(&uri) {
                assert_eq!(
                    prev, dsn,
                    "snapshot {i}: {uri}'s client-computed DSN changed across \
                     reloads ({prev} -> {dsn}) while still in the window — \
                     this is exactly the #629 regression (a segment's DSN must \
                     be stable for as long as it remains in the playlist)"
                );
            }
            seen_dsn.insert(uri, dsn);
        }
    }

    // Sanity: the specific window states named in the original issue are
    // exercised (uris confirm we're looking at the right cuts).
    assert_eq!(extract_uris(&snapshots[0]), vec!["seg0.ts", "seg1.ts"]);
    assert_eq!(extract_uris(&snapshots[1]), vec!["seg1.ts", "seg2.ts"]);
    assert_eq!(extract_uris(&snapshots[2]), vec!["seg2.ts", "seg3.ts"]);
    assert_eq!(extract_uris(&snapshots[3]), vec!["seg3.ts", "seg4.ts"]);
}

fn push_duration_only(seg: &mut StreamingTsHlsSegmenter, id: TrackId, duration: u32) {
    seg.push(
        id,
        Sample::new(vec![0u8; 4], None, None, Some(duration), true).expect("valid timing"),
    )
    .expect("push");
}

fn first_sample_dts(segment: &TsSegment) -> i64 {
    let ir = demux(&segment.bytes);
    ir.tracks[0]
        .samples
        .first()
        .expect("segment carries a sample")
        .timing
        .and_then(|t| t.dts())
        .expect("sample carries a dts")
}

fn first_avc_sample_bytes() -> Vec<u8> {
    let ir = demux(&load_ts());
    ir.tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("fixture carries AVC video")
        .samples[0]
        .data
        .to_vec()
}

fn push_avc(seg: &mut StreamingTsHlsSegmenter, id: TrackId, data: &[u8], duration: u32) {
    seg.push(
        id,
        Sample::new(data.to_vec(), None, None, Some(duration), true).expect("valid timing"),
    )
    .expect("push");
}

fn audio_spec_48k() -> TrackSpec {
    let mut spec = hub_fixtures::audio_track();
    if let CodecConfig::Aac { esds, .. } = &mut spec.config
        && let Some(dc) = esds.es_descriptor.decoder_config.as_mut()
        && let Some(dsi) = dc.decoder_specific_info.as_mut()
    {
        dsi.data = vec![0x11, 0x90];
    }
    spec
}

#[test]
fn set_start_time_moves_a_90_khz_anchor_to_ten_seconds() {
    let spec = hub_fixtures::video_track();
    let id = spec.track_id;
    let data = first_avc_sample_bytes();
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 1, usize::MAX).expect("construct");
    seg.set_start_time(core::time::Duration::from_secs(10));
    push_avc(&mut seg, id, &data, 90_000);
    push_avc(&mut seg, id, &data, 90_000);
    let cut = seg.take_ready();
    assert!(!cut.is_empty(), "the second sample must cut a segment");
    assert_eq!(
        first_sample_dts(&cut[0]),
        909_000,
        "the first PES dts must sit at 10 s in 90 kHz ticks plus the 9000-tick PCR lead"
    );
}

#[test]
fn set_start_time_moves_a_48_khz_audio_anchor_to_ten_seconds() {
    let spec = audio_spec_48k();
    let id = spec.track_id;
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 1, usize::MAX).expect("construct");
    seg.set_start_time(core::time::Duration::from_secs(10));
    push_duration_only(&mut seg, id, 48_000);
    push_duration_only(&mut seg, id, 48_000);
    let cut = seg.take_ready();
    assert!(!cut.is_empty(), "the second sample must cut a segment");
    assert_eq!(
        first_sample_dts(&cut[0]),
        484_800,
        "10 s in 48 kHz ticks (480000) plus the 0.1 s PCR lead (4800)"
    );
}

#[test]
fn set_start_time_after_a_push_is_a_no_op() {
    let spec = hub_fixtures::video_track();
    let id = spec.track_id;
    let data = first_avc_sample_bytes();
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 1, usize::MAX).expect("construct");
    push_avc(&mut seg, id, &data, 90_000);
    seg.set_start_time(core::time::Duration::from_secs(10));
    push_avc(&mut seg, id, &data, 90_000);
    let cut = seg.take_ready();
    assert!(!cut.is_empty());
    assert_eq!(
        first_sample_dts(&cut[0]),
        9_000,
        "a start time set after buffering must not move the timeline (only the PCR lead)"
    );
}

#[test]
fn streaming_playlist_pins_extinf_version_and_target_duration() {
    let spec = hub_fixtures::video_track();
    let id = spec.track_id;
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 2, usize::MAX).expect("construct");
    for i in 0..8 {
        let sync = i == 0 || i == 5 || i == 7;
        let sample =
            Sample::new(vec![0u8; 4], None, None, Some(90_000), sync).expect("valid timing");
        seg.push(id, sample).expect("push");
    }
    seg.finish().expect("finish");
    let playlist = seg.playlist().expect("playlist");

    assert!(playlist.contains("#EXT-X-VERSION:3"), "{playlist}");
    assert!(playlist.contains("#EXT-X-TARGETDURATION:5"), "{playlist}");
    assert!(playlist.contains("#EXTINF:5,"), "{playlist}");
    assert!(playlist.contains("#EXTINF:2,"), "{playlist}");
    assert!(playlist.contains("#EXTINF:1,"), "{playlist}");

    let target: u32 = playlist
        .lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .expect("target duration")
        .trim()
        .parse()
        .unwrap();
    for line in playlist.lines() {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let secs: f64 = rest.trim_end_matches(',').parse().unwrap();
            assert!(
                target as f64 >= secs.ceil(),
                "TARGETDURATION {target} must cover EXTINF {secs}"
            );
        }
    }
}

#[test]
fn demand_saturates_at_the_pending_sample_bound() {
    let spec = hub_fixtures::video_track();
    let id = spec.track_id;
    let mut seg = StreamingTsHlsSegmenter::new(vec![spec], 1, usize::MAX).expect("construct");
    assert!(
        !Stage::demand(&seg).saturated,
        "a fresh segmenter is not full"
    );

    let sample = || Sample::new(vec![0u8; 4], None, None, Some(1), false).expect("valid timing");
    for _ in 0..MAX_PENDING_SAMPLES_PER_TRACK - 1 {
        Stage::feed(&mut seg, (id, sample()), Timestamp::ZERO).expect("feed");
    }
    assert!(
        !Stage::demand(&seg).saturated,
        "MAX-1 buffered samples are not full"
    );
    Stage::feed(&mut seg, (id, sample()), Timestamp::ZERO).expect("feed");
    assert!(
        Stage::demand(&seg).saturated,
        "MAX buffered samples must report saturated"
    );
}
