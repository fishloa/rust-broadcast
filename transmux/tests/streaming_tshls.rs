//! `StreamingTsHlsSegmenter` gate — incremental/live classic-HLS segmentation
//! (issue #571), verified against the existing batch `TsHlsPackager` on the
//! same real fixture (`fixtures/ts/h264_aac.ts`, demuxed via `TsDemux`).
//!
//! Pipeline: `ir = TsDemux(h264_aac.ts)` → push every sample of `ir` through
//! `StreamingTsHlsSegmenter` in global decode-time order → compare the
//! resulting `.ts` segments against `TsHlsPackager::package(&ir)`'s batch
//! output. Byte-identical segments prove the streaming cut/partition logic
//! reproduces the batch algorithm exactly — this cannot be faked by a
//! raw-passthrough implementation, since it requires correct keyframe-aligned
//! cutting *and* correct non-anchor (audio) time-partitioning at each cut.

use broadcast_common::{Package, Unpackage};
use transmux::media::Media;
use transmux::{StreamingTsHlsSegmenter, TrackSpec, TsDemux, TsHlsPackager, TsSegment};

// ── Fixture + pipeline ───────────────────────────────────────────────────────

fn load_ts() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
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
        let scale = t.spec.timescale.max(1) as u64;
        let mut acc: u64 = 0;
        for (i, s) in t.samples.iter().enumerate() {
            let start_secs = acc as f64 / scale as f64;
            items.push((start_secs, tpos, i));
            acc += s.duration as u64;
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
    let mut out = Vec::new();
    for (tpos, i) in merged_decode_order(ir) {
        let track_id = ir.tracks[tpos].spec.track_id;
        let sample = ir.tracks[tpos].samples[i].clone();
        if let Some(s) = seg.push(track_id, sample).expect("push") {
            out.push(s);
        }
    }
    if let Some(s) = seg.finish().expect("finish") {
        out.push(s);
    }
    out
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

    let mut emitted_before_finish: Vec<TsSegment> = Vec::new();
    for (tpos, i) in &order {
        let track_id = ir.tracks[*tpos].spec.track_id;
        let sample = ir.tracks[*tpos].samples[*i].clone();
        if let Some(s) = seg.push(track_id, sample).expect("push") {
            emitted_before_finish.push(s);
        }
    }
    assert!(
        !emitted_before_finish.is_empty(),
        "at least one segment must be available before finish() (progressive emission)"
    );
    assert!(
        emitted_before_finish.len() < batch.segments.len(),
        "not every segment can have been flushed before finish() cuts the trailing partial"
    );

    let mut all: Vec<TsSegment> = emitted_before_finish;
    if let Some(s) = seg.finish().expect("finish") {
        all.push(s);
    }

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

    let mut emitted: Vec<TsSegment> = Vec::new();
    for (tpos, i) in merged_decode_order(&ir) {
        let track_id = ir.tracks[tpos].spec.track_id;
        let sample = ir.tracks[tpos].samples[i].clone();
        if let Some(s) = seg.push(track_id, sample).expect("push") {
            emitted.push(s);
        }
    }
    let m = emitted.len();
    assert!(
        m > window,
        "fixture at 1s target must cut more segments ({m}) than the window ({window})"
    );

    // Still live: no #EXT-X-ENDLIST, window holds exactly the last N segments,
    // and #EXT-X-MEDIA-SEQUENCE has advanced past the rolled-off segments.
    let pl = seg.playlist();
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
    let last = seg.finish().expect("finish");
    let pl2 = seg.playlist();
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
