//! Issue #717 acceptance: `ll-hls-runtime`'s [`TokioClient`] as the
//! **reference client** in the #569 player-validated golden-gate harness.
//!
//! `transmux/tests/golden_gate.rs` (#569) validates only the ORIGIN half of
//! the loop: transmux's own muxer/segmenter output handed straight to an
//! independent decoder (`ffprobe`). This file closes the other half: a real
//! LL-HLS origin -> a real [`TokioClient`] pulling it over loopback HTTP ->
//! the client's *own* reconstructed init + samples muxed back into a real
//! fMP4 -> `ffprobe`. If the client silently drops, duplicates, or reorders a
//! sample, the reconstruction is provably not decodable/matching — not just
//! structurally plausible.
//!
//! # Why the real fixture, not a synthetic SPS/PPS
//!
//! `tests/glass_to_glass.rs` (#717 slice 5's own acceptance test) feeds its
//! origin a placeholder AVC config with a truncated, non-conformant SPS/PPS
//! (`dummy_avc_config`) — fine for its purpose (proving sub-second latency
//! and byte-identical sample round-trip, both checked in this crate without
//! an external decoder), but not decodable by a real one. This harness needs
//! `ffprobe` to genuinely *decode* the reconstruction, so it demuxes the
//! workspace's real captured fixture (`fixtures/ts/h264_aac.ts` — Main
//! profile, 320x240, 25 fps, 75 real video frames, ISO/IEC 13818-1; see
//! `fixtures/ts/CODEC-ORACLE.md`) via `TsDemux` and feeds those real samples
//! through the exact same `LlHlsSegmenter` -> `MediaStore` -> `LlHlsOutput`
//! origin stack `glass_to_glass.rs` uses, live-paced at the fixture's own
//! frame rate. Audio is left out: `multimux::output::llhls::LlHlsOutput`
//! renders a single fixed track (`DEFAULT_TRACK_ID`), exactly as
//! `glass_to_glass.rs` already assumes.
//!
//! # Skip-clean discipline
//!
//! Mirrors `transmux/tests/golden_gate.rs` exactly: every case skips
//! (printing why) rather than failing when `ffprobe` isn't on `PATH`, so
//! `cargo test` stays green without ffmpeg installed. A case that does run
//! must genuinely assert against `ffprobe`'s own output — never just "exited
//! 0".

#![cfg(feature = "tokio")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::Value;

use ll_hls_runtime::client::Output;
use ll_hls_runtime::client::tokio_client::TokioClient;
use ll_hls_runtime::server::DEFAULT_TRACK_ID;
use multimux::origin::{AppState, router};
use multimux::output::Output as MmOutput;
use multimux::output::llhls::LlHlsOutput;
use multimux::store::MediaStore;
use transmux::hls::{MapTag, MediaPlaylist, MediaSegment};
use transmux::ll_hls::LlHlsSegmenter;
use transmux::{CodecConfig, FragmentTrackData, Sample, TrackSpec, TsDemux};

// ── Fixture + scratch-dir plumbing (mirrors transmux/tests/golden_gate.rs) ──

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/golden-gate-tmp")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create golden-gate scratch dir");
    dir
}

/// Demux the real `h264_aac.ts` fixture's AVC video track, forcing its
/// `track_id` to [`DEFAULT_TRACK_ID`] — the fixed single track
/// `multimux::output::llhls::LlHlsOutput` renders regardless of the
/// source's own PID-derived track_id (see `glass_to_glass.rs`'s
/// `TRACK_ID` const, which matches for the same reason).
fn real_video_track_and_samples() -> (TrackSpec, Vec<Sample>) {
    let ts = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
    let media = TsDemux::new().demux(&ts).expect("demux h264_aac.ts");
    let video = media
        .tracks
        .into_iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("h264_aac.ts must carry an AVC video track");
    let mut spec = video.spec;
    spec.track_id = DEFAULT_TRACK_ID;
    (spec, video.samples)
}

// ── External-tool availability + oracle helpers (mirrors transmux's) ───────

fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

macro_rules! skip_unless {
    ($cond:expr, $why:expr) => {
        if !$cond {
            eprintln!("SKIP golden_gate: {}", $why);
            return;
        }
    };
}

fn ffprobe_json(path: &Path) -> Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .expect("spawn ffprobe");
    assert!(
        out.status.success(),
        "ffprobe rejected {}: stderr={}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "ffprobe -v error printed diagnostics for {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("ffprobe -of json must produce valid JSON")
}

/// `ffprobe -count_frames`: an actual libavcodec *decode* of the video
/// stream, not just a demux — the strongest available oracle for "did every
/// sample survive the origin -> client -> reconstruction path intact". A
/// dropped/duplicated/reordered sample breaks H.264 reference structure or
/// changes the count outright; either way this catches it even when the
/// container itself still parses as well-formed.
fn ffprobe_decoded_frame_count(path: &Path) -> u64 {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "default=nokey=1:noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .expect("spawn ffprobe -count_frames");
    assert!(
        out.status.success(),
        "ffprobe -count_frames rejected {}: stderr={}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or_else(|e| {
            panic!(
                "ffprobe -count_frames output not a number: {e} (stdout={:?})",
                String::from_utf8_lossy(&out.stdout)
            )
        })
}

fn stream<'a>(probe: &'a Value, codec_type: &str) -> &'a Value {
    probe["streams"]
        .as_array()
        .expect("streams array")
        .iter()
        .find(|s| s["codec_type"] == codec_type)
        .unwrap_or_else(|| panic!("no {codec_type} stream in ffprobe output: {probe}"))
}

/// Assert `out_probe`'s video identification matches `src_probe`'s — codec
/// name + dimensions — the source fixture's own `ffprobe` identification is
/// the oracle, never a hardcoded literal. Video-only: the LL-HLS origin used
/// here carries only the fixture's video track (see module docs).
fn assert_video_matches_source(src_probe: &Value, out_probe: &Value, what: &str) {
    let src_v = stream(src_probe, "video");
    let out_v = stream(out_probe, "video");
    assert_eq!(
        out_v["codec_name"], src_v["codec_name"],
        "{what}: video codec_name must match source"
    );
    assert_eq!(
        out_v["width"], src_v["width"],
        "{what}: video width must match source"
    );
    assert_eq!(
        out_v["height"], src_v["height"],
        "{what}: video height must match source"
    );
}

// ── LL origin standup (mirrors glass_to_glass.rs's start_ll_origin) ────────

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 120;
const WINDOW_SEGMENTS: usize = 8;

/// Feed every sample in `samples` into `store` via a real
/// `transmux::ll_hls::LlHlsSegmenter`, paced at `h264_aac.ts`'s own real
/// frame rate (25 fps, `fixtures/ts/CODEC-ORACLE.md`) — a live-shaped
/// producer, not a batch dump, exactly as `glass_to_glass.rs`'s own
/// `run_live_producer`.
async fn run_live_producer(store: Arc<MediaStore>, spec: TrackSpec, samples: Vec<Sample>) {
    let track_id = spec.track_id;
    let movie_timescale = spec.timescale;
    let mut seg = LlHlsSegmenter::with_part_target(
        vec![spec],
        movie_timescale,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(seg.init_segment().expect("init segment builds"));

    // h264_aac.ts is a real 25 fps capture (CODEC-ORACLE.md) -> 40ms/frame.
    let frame_interval = Duration::from_millis(40);
    for sample in samples {
        seg.push(track_id, sample).expect("push succeeds");
        for part in seg.take_ready_parts() {
            store.add_part(part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(segment);
        }
        tokio::time::sleep(frame_interval).await;
    }
    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(segment);
    }
}

/// Start the real `multimux` LL-HLS origin on an ephemeral loopback port,
/// serving one stream named `live`.
async fn start_ll_origin(
    spec: TrackSpec,
) -> (Arc<MediaStore>, String, tokio::task::JoinHandle<()>) {
    let store = Arc::new(MediaStore::new(
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
        WINDOW_SEGMENTS,
    ));
    let mut streams = HashMap::new();
    streams.insert(
        "live".to_string(),
        (
            store.clone(),
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn MmOutput>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("listener has a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });
    let _ = spec; // consumed by the caller's producer task instead; kept for symmetry with glass_to_glass's signature shape.
    (store, format!("http://{addr}/live/media.m3u8"), server)
}

// ===========================================================================
// Case 1: the headline #717 acceptance -- LL origin -> real TokioClient over
// loopback -> the client's OWN reconstruction -> ffprobe decodes it, matches
// the source's resolution, and reports exactly the frames fed in.
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ll_hls_client_reference_reconstructs_decodable_stream() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let src_probe = ffprobe_json(&fixture_path());

    let (spec, fed_samples) = real_video_track_and_samples();
    let fed_count = fed_samples.len();
    assert!(
        fed_count > 0,
        "h264_aac.ts must demux to at least one video sample"
    );

    let (store, playlist_url, server) = start_ll_origin(spec.clone()).await;
    let producer = tokio::spawn(run_live_producer(store, spec.clone(), fed_samples.clone()));

    let (init_bytes, got_samples, stats) = tokio::time::timeout(Duration::from_secs(20), async {
        let mut client = TokioClient::new(&playlist_url).expect("client builds");
        let mut init_bytes: Option<Vec<u8>> = None;
        let mut got_samples: Vec<Sample> = Vec::new();
        while got_samples.len() < fed_count {
            match client.next_output().await.expect("client must not error") {
                Some(Output::Init(bytes)) => init_bytes = Some(bytes),
                Some(Output::Samples { samples, .. }) => got_samples.extend(samples),
                Some(Output::EndOfStream) | None => break,
                Some(_other) => {}
            }
        }
        (init_bytes, got_samples, client.stats())
    })
    .await
    .expect("client run must complete within 20s (deadlock/hang guard)");

    producer.await.expect("producer task must not panic");
    server.abort();

    let init_bytes = init_bytes.expect("client must emit an Init before any Samples");

    // Rust-level exact bite: no drops/dupes/reorders, byte-identical samples,
    // in order -- deterministic and independent of ffprobe.
    assert_eq!(
        got_samples.len(),
        fed_count,
        "client must reconstruct every sample the producer pushed, no gaps/duplicates: \
         got {} want {}",
        got_samples.len(),
        fed_count
    );
    for (i, (got, want)) in got_samples.iter().zip(fed_samples.iter()).enumerate() {
        assert_eq!(
            got.data, want.data,
            "sample {i} bytes must round-trip byte-identical"
        );
        assert_eq!(
            got.is_sync, want.is_sync,
            "sample {i} sync flag must round-trip"
        );
    }
    assert!(
        stats.blocking_reloads > 0,
        "the reference-client run must exercise at least one Blocking Playlist Reload \
         (RFC 8216bis, part of #717's player-validated bar): {stats:?}"
    );

    // Mux the CLIENT's own reconstruction (its received Init bytes + the
    // samples it emitted, NOT the origin's) into a real fMP4.
    let media_seg = transmux::build_media_segment(
        1,
        &[FragmentTrackData {
            track_id: DEFAULT_TRACK_ID,
            base_media_decode_time: 0,
            samples: &got_samples,
        }],
    )
    .expect("build a media segment from the client's own reconstructed samples");
    let mut reconstructed = init_bytes;
    reconstructed.extend_from_slice(&media_seg);

    let dir = scratch_dir("ll-hls-golden-gate");
    let path = dir.join("client-reconstructed.mp4");
    std::fs::write(&path, &reconstructed).expect("write reconstructed fMP4");

    // Independent decoder oracle #1: ffprobe identifies it as H.264 at the
    // source's own resolution.
    let out_probe = ffprobe_json(&path);
    assert_video_matches_source(&src_probe, &out_probe, "ll-hls-runtime golden gate");

    // Independent decoder oracle #2: ffprobe's OWN decode (not just demux)
    // reports exactly the frame count fed in -- catches drops/dupes/reorders
    // that corrupt the bitstream even when the container structure alone
    // still looks well-formed.
    let decoded_frames = ffprobe_decoded_frame_count(&path);
    assert_eq!(
        decoded_frames, fed_count as u64,
        "ffprobe's own decode of the client's reconstruction must report exactly the \
         frames fed in, not just a well-formed container"
    );
}

// ===========================================================================
// Self-test: prove the frame-count oracle above actually bites -- dropping
// one sample the way a buggy client would must change ffprobe's own decoded
// count. Without this, `assert_eq!(decoded_frames, fed_count)` above could be
// vacuously true if `ffprobe_decoded_frame_count` silently ignored errors.
// ===========================================================================

#[test]
fn dropped_sample_changes_the_decoded_frame_count() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let (spec, fed_samples) = real_video_track_and_samples();
    let full_count = fed_samples.len();
    assert!(
        full_count > 1,
        "fixture must have more than one sample to drop one meaningfully"
    );

    let init =
        transmux::build_init_segment(&[spec.clone()], spec.timescale).expect("build init segment");

    let dir = scratch_dir("ll-hls-golden-gate-mutation");

    let full_media_seg = transmux::build_media_segment(
        1,
        &[FragmentTrackData {
            track_id: spec.track_id,
            base_media_decode_time: 0,
            samples: &fed_samples,
        }],
    )
    .expect("build full media segment");
    let mut full = init.clone();
    full.extend_from_slice(&full_media_seg);
    let full_path = dir.join("full.mp4");
    std::fs::write(&full_path, &full).unwrap();
    let full_decoded = ffprobe_decoded_frame_count(&full_path);
    assert_eq!(
        full_decoded, full_count as u64,
        "sanity: the un-mutated reconstruction must decode to exactly the fed sample count"
    );

    // Drop the LAST sample only, keeping every earlier frame's reference
    // structure intact -- isolates the frame-count bite from any unrelated
    // decode-error side effect a mid-stream drop could also cause.
    let dropped: Vec<Sample> = fed_samples[..full_count - 1].to_vec();
    let dropped_media_seg = transmux::build_media_segment(
        1,
        &[FragmentTrackData {
            track_id: spec.track_id,
            base_media_decode_time: 0,
            samples: &dropped,
        }],
    )
    .expect("build media segment missing one sample");
    let mut truncated = init;
    truncated.extend_from_slice(&dropped_media_seg);
    let truncated_path = dir.join("dropped.mp4");
    std::fs::write(&truncated_path, &truncated).unwrap();
    let truncated_decoded = ffprobe_decoded_frame_count(&truncated_path);

    assert_eq!(
        truncated_decoded,
        full_count as u64 - 1,
        "dropping one sample must reduce ffprobe's own decoded frame count by exactly one -- \
         if it doesn't, the golden gate's frame-count assertion above is not biting"
    );
}

// ===========================================================================
// Case 2: non-LL / full-segment fallback path also decodes (cheap: reuses
// the same real fixture + client + ffprobe helpers as Case 1, over a
// hand-rolled non-LL axum origin, mirroring glass_to_glass.rs's own
// non_ll_origin_plays_via_full_segment_fallback_over_http).
// ===========================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_ll_full_segment_path_also_decodes() {
    skip_unless!(ffprobe_available(), "ffprobe not on PATH");

    let src_probe = ffprobe_json(&fixture_path());
    let (spec, fed_samples) = real_video_track_and_samples();
    let fed_count = fed_samples.len();

    // A single full segment holding every sample: a target/part duration far
    // longer than the fixture's ~3s runtime means nothing auto-splits until
    // the trailing `flush()` closes the one and only segment.
    let mut seg =
        LlHlsSegmenter::with_part_target(vec![spec.clone()], spec.timescale, 3600.0, 3_600_000)
            .expect("segmenter builds");
    for sample in fed_samples.clone() {
        seg.push(spec.track_id, sample).expect("push succeeds");
    }
    seg.flush().expect("flush succeeds");
    let init_bytes = seg.init_segment().expect("init segment builds");
    let seg1 = seg
        .take_ready_segments()
        .into_iter()
        .next()
        .expect("the one segment must have closed on flush");

    let playlist = MediaPlaylist {
        version: 3,
        target_duration: 4,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg1.m4s".to_string(),
            duration: seg1.duration,
            discontinuous: false,
            parts: vec![],
            byte_range: None,
            map: Some(MapTag {
                uri: "init.mp4".to_string(),
                byte_range: None,
            }),
        }],
        open_segment: None,
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
        rendition_reports: vec![],
        skip: None,
    };
    let playlist_text = playlist.to_m3u8();
    assert!(
        !playlist_text.contains("#EXT-X-PART"),
        "the fixture playlist must genuinely carry no PART tags:\n{playlist_text}"
    );

    let init_for_route = init_bytes.clone();
    let seg1_bytes_for_route = seg1.bytes.clone();
    let app = Router::new()
        .route(
            "/media.m3u8",
            get(move || {
                let text = playlist_text.clone();
                async move { text.into_response() }
            }),
        )
        .route(
            "/init.mp4",
            get(move || {
                let bytes = init_for_route.clone();
                async move { bytes.into_response() }
            }),
        )
        .route(
            "/seg1.m4s",
            get(move || {
                let bytes = seg1_bytes_for_route.clone();
                async move { bytes.into_response() }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("listener has a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });

    let (got_init, got_samples) = tokio::time::timeout(Duration::from_secs(10), async {
        let mut client =
            TokioClient::new(format!("http://{addr}/media.m3u8")).expect("client builds");
        let mut got_init: Option<Vec<u8>> = None;
        let mut got_samples: Vec<Sample> = Vec::new();
        loop {
            match client.next_output().await.expect("client must not error") {
                Some(Output::Init(bytes)) => got_init = Some(bytes),
                Some(Output::Samples { samples, .. }) => got_samples.extend(samples),
                Some(Output::EndOfStream) | None => break,
                Some(_other) => {}
            }
        }
        (got_init, got_samples)
    })
    .await
    .expect("non-LL client run must complete within 10s (deadlock/hang guard)");

    server.abort();

    let got_init = got_init.expect("must emit init even on the fallback path");
    assert_eq!(
        got_init, init_bytes,
        "init bytes must round-trip byte-identical on the fallback path"
    );
    assert_eq!(
        got_samples.len(),
        fed_count,
        "fallback path must reconstruct every sample, no gaps/duplicates"
    );
    for (i, (got, want)) in got_samples.iter().zip(fed_samples.iter()).enumerate() {
        assert_eq!(
            got.data, want.data,
            "sample {i} must round-trip byte-identical on the fallback path"
        );
    }

    let media_seg = transmux::build_media_segment(
        1,
        &[FragmentTrackData {
            track_id: spec.track_id,
            base_media_decode_time: 0,
            samples: &got_samples,
        }],
    )
    .expect("build media segment from fallback-path samples");
    let mut reconstructed = got_init;
    reconstructed.extend_from_slice(&media_seg);

    let dir = scratch_dir("ll-hls-golden-gate-fallback");
    let path = dir.join("client-reconstructed-fallback.mp4");
    std::fs::write(&path, &reconstructed).expect("write reconstructed fMP4");

    let out_probe = ffprobe_json(&path);
    assert_video_matches_source(
        &src_probe,
        &out_probe,
        "ll-hls-runtime golden gate (non-LL fallback)",
    );

    let decoded_frames = ffprobe_decoded_frame_count(&path);
    assert_eq!(
        decoded_frames, fed_count as u64,
        "ffprobe's own decode of the fallback-path reconstruction must report exactly the \
         frames fed in"
    );
}
