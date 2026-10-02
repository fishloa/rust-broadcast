//! Integration tests for the Smooth Streaming output (issue #742).
//!
//! Test 1: The manifest is served and is well-formed — parses as
//!   `SmoothManifest`, has quality levels and track entries, durations are
//!   non-zero.
//! Test 2: Advertised == servable — parse the manifest, extract fragment
//!   URLs, request them, assert real bytes come back.
//! Test 3: Auth is enforced — unauthenticated requests to manifest and
//!   fragments are rejected on a route with output auth.
//! Test 4: A route with `["llhls","smooth"]` serves both from one ingest,
//!   proving shared Trunk.
//! Test 5: Config round-trip — `"smooth"` deserialises and validates.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_auth::Credentials;
use tower::ServiceExt;

use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::output::smooth::SmoothOutput;
use multimux::route::RouteHandle;
use transmux::avc_config_from_sprop;
use transmux::ll_hls::LlHlsSegmenter;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::smooth_parse::SmoothManifest;

/// A real-ish sprop-parameter-sets pair (SPS+PPS) — same one used by
/// `multimux::source::rtsp`'s own tests.
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
/// 90 kHz video timescale.
const VIDEO_TIMESCALE: u32 = 90_000;
const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 500;

fn video_track_spec() -> TrackSpec {
    let config = avc_config_from_sprop(SPROP).expect("valid sprop");
    TrackSpec::new(
        1,
        VIDEO_TIMESCALE,
        CodecConfig::Avc {
            config,
            width: 1280,
            height: 720,
        },
    )
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("well-formed GET request")
}

/// Feed video frames through a real `LlHlsSegmenter` into `store`,
/// publishing the SPTS program.
fn feed_via_segmenter(
    store: &RouteHandle,
    specs: Vec<TrackSpec>,
    batches: Vec<Vec<(u32, Sample)>>,
) {
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);

    let mut seg = LlHlsSegmenter::with_part_target(
        specs.clone(),
        transmux::VIDEO_CLOCK_RATE,
        TARGET_DURATION_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter builds");
    store.set_init(program, seg.init_segment().expect("init segment builds"));
    store.set_track_specs(program, specs);

    for batch in batches {
        for (track_id, sample) in batch {
            seg.push(track_id, sample).expect("push succeeds");
        }
        for part in seg.take_ready_parts() {
            store.add_part(program, part);
        }
        for segment in seg.take_ready_segments() {
            store.add_segment(program, segment).expect("add_segment");
        }
    }

    seg.flush().expect("flush succeeds");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add_segment");
    }
}

/// A route driven one segment at a time through a kept-alive
/// `LlHlsSegmenter`, so a test can render the manifest, slide the window (add
/// another segment), and render again.
struct SlideRoute {
    store: Arc<RouteHandle>,
    seg: LlHlsSegmenter,
    next_frame: u32,
}

impl SlideRoute {
    /// Build a route whose target is `W13_TARGET_SECS` (4 s) per segment.
    fn new() -> Self {
        let store = Arc::new(RouteHandle::new(W13_TARGET_SECS, PART_TARGET_MS, 2));
        let specs = vec![video_track_spec()];
        let program = media_plane::ProgramId(0);
        store.publish_new_program(program);
        let seg = LlHlsSegmenter::with_part_target(
            specs.clone(),
            transmux::VIDEO_CLOCK_RATE,
            W13_TARGET_SECS,
            PART_TARGET_MS,
        )
        .expect("segmenter builds");
        store.set_init(program, seg.init_segment().expect("init"));
        store.set_track_specs(program, specs);
        Self {
            store,
            seg,
            next_frame: 0,
        }
    }

    /// Push the frames of one more 4 s segment (a sync frame every
    /// `W13_SYNC_EVERY`) and publish whatever it closed.
    fn push_segment(&mut self) {
        let program = media_plane::ProgramId(0);
        // Push one frame past the segment boundary so the segmenter closes the
        // segment (the cut is triggered by the *next* sync frame).
        for _ in 0..=W13_SYNC_EVERY {
            let i = self.next_frame;
            let is_sync = i.is_multiple_of(W13_SYNC_EVERY);
            let sample = Sample::new(
                vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
                Some(i64::from(i) * i64::from(W13_FRAME_DUR)),
                Some(i64::from(i) * i64::from(W13_FRAME_DUR)),
                Some(W13_FRAME_DUR),
                is_sync,
            );
            self.seg.push(1, sample).expect("push");
            self.next_frame += 1;
        }
        for part in self.seg.take_ready_parts() {
            self.store.add_part(program, part);
        }
        for segment in self.seg.take_ready_segments() {
            self.store
                .add_segment(program, segment)
                .expect("add_segment");
        }
    }
}

/// Build an axum app from a route handle with Smooth output.
fn build_app(store: Arc<RouteHandle>, output: Arc<dyn Output>) -> axum::Router {
    let mut streams = HashMap::new();
    streams.insert("cam".to_string(), (store.clone(), vec![output]));
    router(Arc::new(AppState::new(streams)))
}

/// Test 1: The manifest is served and is well-formed.
#[tokio::test]
async fn manifest_is_served_and_well_formed() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    // 60 frames = ~2s of video, enough for 2 segments.
    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let app = build_app(store.clone(), Arc::new(SmoothOutput));

    // Request the manifest
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);

    let manifest_xml = body_string(resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml)
        .unwrap_or_else(|e| panic!("manifest parse failed: {e:?}\nmanifest:\n{manifest_xml}"));

    assert_eq!(manifest.major_version, 2);
    assert!(
        !manifest.streams.is_empty(),
        "must have at least one StreamIndex"
    );
    assert!(
        manifest.duration.unwrap_or(0) > 0,
        "duration must be non-zero"
    );

    let stream = &manifest.streams[0];
    assert_eq!(
        stream.stream_type,
        transmux::smooth_parse::StreamType::Video
    );
    assert!(
        !stream.qualities.is_empty(),
        "must have at least one QualityLevel"
    );
    assert!(
        stream.chunks.unwrap_or(0) > 0,
        "must have at least one chunk"
    );

    println!(
        "manifest parse OK: {} streams, chunks: {:?}",
        manifest.streams.len(),
        manifest.streams[0].chunks
    );
}

/// Test 2: Advertised == servable — parse the manifest, extract the fragment
/// URLs it actually advertises, request each one, assert real bytes come back.
#[tokio::test]
async fn advertised_equals_servable() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let app = build_app(store.clone(), Arc::new(SmoothOutput));

    // Get manifest
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(resp.status(), StatusCode::OK);
    let manifest_xml = body_string(resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("manifest must parse");

    // Derive fragment URLs from the manifest
    for stream in &manifest.streams {
        let chunks = stream
            .enumerate_chunks()
            .unwrap_or_else(|e| panic!("enumerate_chunks failed: {e:?}"));
        assert!(
            !chunks.is_empty(),
            "manifest must advertise at least one chunk"
        );

        // Use the first quality level's bitrate
        let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);

        for (start_time, _duration) in &chunks {
            let fragment_url = stream.resolve_fragment_url(bitrate, *start_time);
            let uri = format!("/cam/{fragment_url}");

            let frag_resp = app
                .clone()
                .oneshot(get(&uri))
                .await
                .unwrap_or_else(|e| panic!("fragment request failed for {uri}: {e}"));

            assert_eq!(
                frag_resp.status(),
                StatusCode::OK,
                "fragment {uri} must return 200 OK"
            );

            let fragment_bytes = body_bytes(frag_resp).await;
            assert!(
                !fragment_bytes.is_empty(),
                "fragment {uri} must return non-empty bytes"
            );
        }
    }
}

/// Test 3: Auth is enforced — unauthenticated requests to manifest and
/// fragments are rejected.
#[tokio::test]
async fn auth_is_enforced() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..30u32 {
        let sample = Sample::new(
            vec![0xAA; 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            i % 30 == 0,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    // Build an app with output auth (Basic, user:pass)
    let verifier = Arc::new(broadcast_auth::Verifier::new(
        Credentials::Basic {
            username: "viewer".into(),
            password: "secret".into(),
        },
        "smooth-test",
    ));

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![Arc::new(SmoothOutput) as Arc<dyn Output>],
        ),
    );
    let app = router(Arc::new(AppState::new(streams).with_output_auth(verifier)));

    // Manifest: no auth -> 401
    let resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("router call");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "manifest without auth must return 401"
    );

    // Fragment: no auth -> 401 (need a valid fragment URL first from an authed manifest)
    // First, get a manifest with auth to find a fragment URL
    let auth_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/cam/Manifest")
                .header("Authorization", "Basic dmlld2VyOnNlY3JldA==") // viewer:secret
                .body(Body::empty())
                .expect("auth request"),
        )
        .await
        .expect("authed manifest call");
    assert_eq!(auth_resp.status(), StatusCode::OK);
    let manifest_xml = body_string(auth_resp).await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("parse manifest");

    let stream = &manifest.streams[0];
    let chunks = stream.enumerate_chunks().expect("enumerate_chunks");
    let (start_time, _) = chunks[0];
    let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
    let fragment_url = stream.resolve_fragment_url(bitrate, start_time);
    let fragment_uri = format!("/cam/{fragment_url}");

    // Fragment without auth -> 401
    let frag_resp = app
        .clone()
        .oneshot(get(&fragment_uri))
        .await
        .expect("fragment call");
    assert_eq!(
        frag_resp.status(),
        StatusCode::UNAUTHORIZED,
        "fragment without auth must return 401"
    );
}

/// Test 4: A route with `["llhls","smooth"]` serves both from one ingest,
/// proving they share the Trunk.
#[tokio::test]
async fn llhls_and_smooth_serve_from_same_trunk() {
    let store = Arc::new(RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, 8));
    let specs = vec![video_track_spec()];

    let mut batches = Vec::new();
    for i in 0..60u32 {
        let is_sync = i % 30 == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);

    let mut streams = HashMap::new();
    streams.insert(
        "cam".to_string(),
        (
            store.clone(),
            vec![
                Arc::new(LlHlsOutput::default()) as Arc<dyn Output>,
                Arc::new(SmoothOutput) as Arc<dyn Output>,
            ],
        ),
    );
    let app = router(Arc::new(AppState::new(streams)));

    // LL-HLS media playlist works
    let hls_resp = app
        .clone()
        .oneshot(get("/cam/media.m3u8"))
        .await
        .expect("hls router call");
    assert_eq!(hls_resp.status(), StatusCode::OK);
    let playlist = body_string(hls_resp).await;
    assert!(playlist.contains("#EXTM3U"), "must be valid HLS playlist");

    // Smooth manifest works
    let smooth_resp = app
        .clone()
        .oneshot(get("/cam/Manifest"))
        .await
        .expect("smooth router call");
    assert_eq!(smooth_resp.status(), StatusCode::OK);
    let manifest_xml = body_string(smooth_resp).await;
    assert!(
        manifest_xml.contains("<SmoothStreamingMedia"),
        "must be valid Smooth manifest"
    );

    // Smooth fragment works
    let manifest = SmoothManifest::parse(&manifest_xml).expect("parse manifest");
    let stream = &manifest.streams[0];
    let chunks = stream.enumerate_chunks().expect("enumerate_chunks");
    let (start_time, _) = chunks[0];
    let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
    let fragment_url = stream.resolve_fragment_url(bitrate, start_time);
    let fragment_uri = format!("/cam/{fragment_url}");

    let frag_resp = app
        .clone()
        .oneshot(get(&fragment_uri))
        .await
        .expect("fragment call");
    assert_eq!(frag_resp.status(), StatusCode::OK);
    let frag_bytes = body_bytes(frag_resp).await;
    assert!(!frag_bytes.is_empty(), "fragment bytes non-empty");
}

// ---------------------------------------------------------------------------
// W13a/b oracles: fragment duration/tfxd vs the manifest timeline, and one
// segment == one fragment. A real `LlHlsSegmenter` (real SPS/PPS) cuts >= 4 s
// segments; the served fragments are then read by `mp4dump`/`MP4Box`.
// ---------------------------------------------------------------------------

/// 4-second target so the segmenter produces the >= 4 s segments W13a is
/// about; 12 s of video at 30 fps (sync every 120 frames = 4 s) → 3 segments.
const W13_TARGET_SECS: f64 = 4.0;
const W13_FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
const W13_SYNC_EVERY: u32 = 120;
const W13_FRAMES: u32 = W13_SYNC_EVERY * 3 + 10;

fn have_oracle(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("-version")
        .output()
        .or_else(|_| std::process::Command::new(bin).arg("-h").output())
        .map(|_| true)
        .unwrap_or(false)
}

/// A route with `W13_TARGET_SECS` segments: `W13_FRAMES` video frames through
/// a real `LlHlsSegmenter`.
fn w13_route() -> Arc<RouteHandle> {
    let store = Arc::new(RouteHandle::new(W13_TARGET_SECS, PART_TARGET_MS, 16));
    let specs = vec![video_track_spec()];
    let mut batches = Vec::new();
    for i in 0..W13_FRAMES {
        let is_sync = i % W13_SYNC_EVERY == 0;
        let sample = Sample::new(
            vec![0xAAu8.wrapping_add((i % 251) as u8); 64],
            Some(i64::from(i) * i64::from(W13_FRAME_DUR)),
            Some(i64::from(i) * i64::from(W13_FRAME_DUR)),
            Some(W13_FRAME_DUR),
            is_sync,
        );
        batches.push(vec![(1u32, sample)]);
    }
    feed_via_segmenter(&store, specs, batches);
    store
}

/// W13a/b: for the 2nd and 3rd video fragments, `mp4dump` must report the
/// same `fragment_absolute_time` (tfxd) the manifest `c@t` advertises, and a
/// `fragment_duration` equal to `c@d` — i.e. the fragment's own timeline is
/// the manifest's, not a per-segment re-zero.
#[tokio::test]
async fn w13_fragment_tfxd_and_duration_match_manifest() {
    // Uses raw bytes only (no `mp4dump`), so it never skips.
    let store = w13_route();
    let app = build_app(store, Arc::new(SmoothOutput));

    let manifest_xml = body_string(
        app.clone()
            .oneshot(get("/cam/Manifest"))
            .await
            .expect("router"),
    )
    .await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("manifest parses");
    let video = manifest
        .streams
        .iter()
        .find(|s| s.stream_type.name() == "video")
        .expect("video StreamIndex");
    let chunks = video.enumerate_chunks().expect("chunks");
    assert!(
        chunks.len() >= 3,
        "need >= 3 chunks for a non-zero 2nd/3rd t: {chunks:?}"
    );
    let bitrate = video.qualities.first().map(|q| q.bitrate).unwrap_or(0);

    for (i, (start_time, duration)) in chunks.iter().enumerate().take(3) {
        let url = video.resolve_fragment_url(bitrate, *start_time);
        let resp = app
            .clone()
            .oneshot(get(&format!("/cam/{url}")))
            .await
            .expect("router");
        let status = resp.status();
        let bytes = body_bytes(resp).await;
        assert_eq!(status, StatusCode::OK, "chunk {i} url {url}");
        assert!(!bytes.is_empty(), "chunk {i} fragment non-empty");

        // `mp4dump`/`MP4Box` do not descend into the `tfxd` `uuid` box, so
        // read it from the raw wire bytes: locate the TFXD UUID
        // (MS-SSTR §2.2.4.4) and parse its FullBox payload directly — an
        // independent reader that does not touch this crate's packager.
        let (tfxd_time, tfxd_dur) =
            read_tfxd(&bytes).unwrap_or_else(|| panic!("chunk {i}: no tfxd box in fragment"));
        assert_eq!(
            tfxd_time, *start_time,
            "chunk {i}: tfxd absolute time must equal manifest c@t"
        );
        // The fragment's own `trun` durations must sum to the manifest `c@d`
        // exactly (a client rebuilds its timeline from tfxd + Σtrun).
        let sum_trun: u64 = read_trun_durations(&bytes).iter().sum();
        assert_eq!(
            sum_trun, *duration,
            "chunk {i}: Σtrun durations must equal manifest c@d"
        );
        assert_eq!(
            tfxd_dur, *duration,
            "chunk {i}: tfxd duration must equal manifest c@d"
        );
        // And the next chunk starts exactly where this one ends:
        // tfxd(n) + Σtrun(n) == tfxd(n+1) == c@t(n+1).
        if let Some((next_t, _)) = chunks.get(i + 1) {
            assert_eq!(
                tfxd_time.saturating_add(sum_trun),
                *next_t,
                "chunk {i}: tfxd + Σtrun must equal the next chunk's c@t"
            );
        }
    }
}

/// W13 follow-up, real fixture: an H.264 + AAC (44.1 kHz) capture, cut into
/// >= 4 s segments, with **per-track** per-chunk checksum invariants —
/// `tfxd(chunk) == c@t`, `Σtrun(chunk) == c@d`, and
/// `tfxd(n) + Σtrun(n) == c@t(n+1)` — for both the video and the audio
/// StreamIndex. AAC's 1024/44100 s frames do not sum to the video segment's
/// rounded duration, so a shared muxed `c@d` would gap/overlap; each track's
/// own timeline is required.
#[tokio::test]
async fn w13_real_multitrack_per_chunk_timeline_is_exact() {
    let ts = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac_40s.ts"
    ))
    .expect("h264_aac_40s.ts fixture must exist");
    let media = transmux::TsDemux::new().demux(&ts).expect("demux");
    let specs: Vec<TrackSpec> = media
        .tracks
        .iter()
        .filter(|t| {
            matches!(
                t.spec.config,
                CodecConfig::Avc { .. } | CodecConfig::Aac { .. }
            )
        })
        .map(|t| t.spec.clone())
        .collect();
    assert_eq!(specs.len(), 2, "one video + one audio track");

    let store = Arc::new(RouteHandle::new(W13_TARGET_SECS, PART_TARGET_MS, 16));
    let program = media_plane::ProgramId(0);
    store.publish_new_program(program);
    let mut seg = LlHlsSegmenter::with_part_target(
        specs.clone(),
        transmux::VIDEO_CLOCK_RATE,
        W13_TARGET_SECS,
        PART_TARGET_MS,
    )
    .expect("segmenter");
    store.set_init(program, seg.init_segment().expect("init"));
    store.set_track_specs(program, specs.clone());
    // Feed samples interleaved in decode-time order across both tracks — a
    // per-track greedy push would buffer all of a track until `flush`, so one
    // segment would carry the whole track.
    let mut events: Vec<(i64, u32, Sample)> = Vec::new();
    for track in &media.tracks {
        if specs.iter().any(|s| s.track_id == track.spec.track_id) {
            for sample in &track.samples {
                events.push((sample.dts.unwrap_or(0), track.spec.track_id, sample.clone()));
            }
        }
    }
    events.sort_by_key(|(dts, _, _)| *dts);
    for (_, track_id, sample) in events {
        seg.push(track_id, sample).expect("push");
    }
    seg.flush().expect("flush");
    for part in seg.take_ready_parts() {
        store.add_part(program, part);
    }
    for segment in seg.take_ready_segments() {
        store.add_segment(program, segment).expect("add_segment");
    }

    let app = build_app(store, Arc::new(SmoothOutput));
    let manifest_xml = body_string(
        app.clone()
            .oneshot(get("/cam/Manifest"))
            .await
            .expect("router"),
    )
    .await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("parse");
    assert_eq!(manifest.streams.len(), 2, "video + audio StreamIndexes");

    for stream in &manifest.streams {
        let chunks = stream.enumerate_chunks().expect("chunks");
        assert!(
            chunks.len() >= 2,
            "{} track must have >= 2 chunks: {chunks:?}",
            stream.stream_type.name()
        );
        let bitrate = stream.qualities.first().map(|q| q.bitrate).unwrap_or(0);
        for (i, (t, d)) in chunks.iter().enumerate() {
            let url = stream.resolve_fragment_url(bitrate, *t);
            let bytes = body_bytes(
                app.clone()
                    .oneshot(get(&format!("/cam/{url}")))
                    .await
                    .expect("router"),
            )
            .await;
            let (tfxd, tfxd_dur) = read_tfxd(&bytes)
                .unwrap_or_else(|| panic!("{} chunk {i}: no tfxd", stream.stream_type.name()));
            let sum_trun: u64 = read_trun_durations(&bytes).iter().sum();
            assert_eq!(
                tfxd,
                *t,
                "{} chunk {i}: tfxd == c@t",
                stream.stream_type.name()
            );
            assert_eq!(
                sum_trun,
                *d,
                "{} chunk {i}: Σtrun == c@d",
                stream.stream_type.name()
            );
            assert_eq!(
                tfxd_dur,
                *d,
                "{} chunk {i}: tfxd duration == c@d",
                stream.stream_type.name()
            );
            if let Some((next_t, _)) = chunks.get(i + 1) {
                assert_eq!(
                    tfxd.saturating_add(sum_trun),
                    *next_t,
                    "{} chunk {i}: tfxd + Σtrun == c@t(n+1)",
                    stream.stream_type.name()
                );
            }
        }
    }
}

/// W13 follow-up: the manifest `c@t`/`c@d` and the served fragment bytes for
/// a given segment must be **identical before and after the window slides**.
/// A window-relative `t` (cumulative from the current first chunk) would
/// change on every slide — a Smooth client sees a non-monotonic timeline. The
/// fix keys everything on each chunk's absolute position on the track
/// timeline.
#[tokio::test]
async fn w13_manifest_and_fragment_are_stable_across_a_window_slide() {
    let mut route = SlideRoute::new();
    let app = build_app(Arc::clone(&route.store), Arc::new(SmoothOutput));

    // Render with one segment in the window.
    route.push_segment();
    let before_xml = body_string(
        app.clone()
            .oneshot(get("/cam/Manifest"))
            .await
            .expect("router"),
    )
    .await;
    let before = SmoothManifest::parse(&before_xml).expect("parse");
    let before_video = before
        .streams
        .iter()
        .find(|s| s.stream_type.name() == "video")
        .expect("video");
    let before_chunks = before_video.enumerate_chunks().expect("chunks");
    assert_eq!(before_chunks.len(), 1, "one segment, one chunk");
    let bitrate = before_video
        .qualities
        .first()
        .map(|q| q.bitrate)
        .unwrap_or(0);

    // Fetch the first fragment.
    let url = before_video.resolve_fragment_url(bitrate, before_chunks[0].0);
    let before_frag = body_bytes(
        app.clone()
            .oneshot(get(&format!("/cam/{url}")))
            .await
            .expect("router"),
    )
    .await;

    // Slide: add a second segment (the window cap is 2).
    route.push_segment();
    let after_xml = body_string(
        app.clone()
            .oneshot(get("/cam/Manifest"))
            .await
            .expect("router"),
    )
    .await;
    let after = SmoothManifest::parse(&after_xml).expect("parse");
    let after_video = after
        .streams
        .iter()
        .find(|s| s.stream_type.name() == "video")
        .expect("video");
    let after_chunks = after_video.enumerate_chunks().expect("chunks");
    assert_eq!(after_chunks.len(), 2, "two segments after the slide");

    // The surviving (first) chunk keeps its exact `t`/`d`.
    assert_eq!(
        after_chunks[0], before_chunks[0],
        "the surviving chunk's c@t/c@d must be identical after the slide (before {:?}, after {:?})",
        before_chunks[0], after_chunks[0]
    );

    // The fragment served for the same `t` is byte-identical.
    let after_url = after_video.resolve_fragment_url(bitrate, before_chunks[0].0);
    let after_frag = body_bytes(
        app.clone()
            .oneshot(get(&format!("/cam/{after_url}")))
            .await
            .expect("router"),
    )
    .await;
    assert_eq!(
        before_frag, after_frag,
        "the fragment for the same absolute time must be byte-identical after the slide"
    );
    assert!(!after_frag.is_empty(), "fragment non-empty");
}

/// The `tfxd` box's extended-type UUID (MS-SSTR §2.2.4.4:
/// `6d1d9b05-42d5-44e6-80e2-141daff757b2`), hard-coded from the spec rather
/// than imported, so this reader shares no code with the packager.
const TFXD_UUID_LITERAL: [u8; 16] = [
    0x6d, 0x1d, 0x9b, 0x05, 0x42, 0xd5, 0x44, 0xe6, 0x80, 0xe2, 0x14, 0x1d, 0xaf, 0xf7, 0x57, 0xb2,
];

/// Read `(FragmentAbsoluteTime, FragmentDuration)` from a fragment's `tfxd`
/// box by scanning the raw bytes for its UUID and parsing the following
/// FullBox. `None` if the UUID is absent or the box is truncated.
fn read_tfxd(fragment: &[u8]) -> Option<(u64, u64)> {
    // FullBox(u8 version + 24-bit flags) = 4 bytes, then two u64 fields.
    let uuid_at = fragment
        .windows(TFXD_UUID_LITERAL.len())
        .position(|w| w == TFXD_UUID_LITERAL)?;
    let after_uuid = uuid_at + TFXD_UUID_LITERAL.len();
    // Skip the 4-byte FullBox header.
    let body = fragment.get(after_uuid + 4..after_uuid + 4 + 16)?;
    let absolute = u64::from_be_bytes(body[..8].try_into().ok()?);
    let duration = u64::from_be_bytes(body[8..16].try_into().ok()?);
    Some((absolute, duration))
}

/// Read every `trun` sample-duration in a fragment (the `moof` → `traf` →
/// `trun` box, honoring the `sample-duration-present` flag), by walking the
/// raw ISO-BMFF boxes. Independent of the packager.
fn read_trun_durations(fragment: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    walk_boxes(fragment, &mut out);
    out
}

fn walk_boxes(bytes: &[u8], out: &mut Vec<u64>) {
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap_or([0; 4])) as usize;
        let ty = &bytes[pos + 4..pos + 8];
        if size < 8 || pos + size > bytes.len() {
            break;
        }
        let body = &bytes[pos + 8..pos + size];
        match ty {
            b"moof" => walk_boxes(body, out),
            b"traf" => walk_boxes(body, out),
            b"trun" => read_one_trun(body, out),
            _ => {}
        }
        pos += size;
    }
}

fn read_one_trun(body: &[u8], out: &mut Vec<u64>) {
    const FLAG_DATA_OFFSET: u32 = 0x0000_0001;
    const FLAG_FIRST_SAMPLE_FLAGS: u32 = 0x0000_0004;
    const FLAG_SAMPLE_DURATION: u32 = 0x0000_0100;
    const FLAG_SAMPLE_SIZE: u32 = 0x0000_0200;
    const FLAG_SAMPLE_FLAGS: u32 = 0x0000_0400;
    const FLAG_SAMPLE_CTO: u32 = 0x0000_0800;
    if body.len() < 8 {
        return;
    }
    let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
    let sample_count = u32::from_be_bytes(body[4..8].try_into().unwrap_or([0; 4])) as usize;
    let mut cur = 8usize;
    if flags & FLAG_DATA_OFFSET != 0 {
        cur += 4;
    }
    if flags & FLAG_FIRST_SAMPLE_FLAGS != 0 {
        cur += 4;
    }
    for _ in 0..sample_count {
        if flags & FLAG_SAMPLE_DURATION != 0 {
            if cur + 4 > body.len() {
                break;
            }
            out.push(u64::from(u32::from_be_bytes(
                body[cur..cur + 4].try_into().unwrap_or([0; 4]),
            )));
            cur += 4;
        }
        if flags & FLAG_SAMPLE_SIZE != 0 {
            cur += 4;
        }
        if flags & FLAG_SAMPLE_FLAGS != 0 {
            cur += 4;
        }
        if flags & FLAG_SAMPLE_CTO != 0 {
            cur += 4;
        }
    }
}

/// W13a: a >= 4 s segment is a **single** fragment carrying every sample —
/// the pre-fix `SmoothPackager::default()` (2 s target) split it and returned
/// only the first fragment, dropping the rest.
#[tokio::test]
async fn w13_one_segment_is_one_fragment() {
    if !have_oracle("mp4dump") {
        eprintln!("SKIP w13_one_segment_is_one_fragment: mp4dump not on PATH");
        return;
    }
    let store = w13_route();
    let app = build_app(store, Arc::new(SmoothOutput));
    let manifest_xml = body_string(
        app.clone()
            .oneshot(get("/cam/Manifest"))
            .await
            .expect("router"),
    )
    .await;
    let manifest = SmoothManifest::parse(&manifest_xml).expect("manifest parses");
    let video = manifest
        .streams
        .iter()
        .find(|s| s.stream_type.name() == "video")
        .expect("video StreamIndex");
    let chunks = video.enumerate_chunks().expect("chunks");
    let bitrate = video.qualities.first().map(|q| q.bitrate).unwrap_or(0);

    // One video fragment per chunk, each carrying the whole segment's
    // samples. The 12 s / 4 s cut yields 3 fragments of 120 frames each
    // (the tail 10 frames ride the last fragment).
    let mut frames = 0u64;
    for (i, (start_time, _)) in chunks.iter().enumerate() {
        let url = video.resolve_fragment_url(bitrate, *start_time);
        let bytes = body_bytes(
            app.clone()
                .oneshot(get(&format!("/cam/{url}")))
                .await
                .expect("router"),
        )
        .await;
        let dumped = mp4dump(&bytes);
        let samples = dumped_u64_after(&dumped, "sample count")
            .unwrap_or_else(|| panic!("chunk {i}: no sample_count in:\n{dumped}"));
        assert!(
            samples > 0,
            "chunk {i}: fragment must carry samples (non-empty mdat)"
        );
        frames += samples;
    }
    assert_eq!(
        frames,
        u64::from(W13_FRAMES),
        "the fragments together must carry every frame, none dropped"
    );
}

/// `mp4dump --verbosity 2` output for `fragment`, written to a temp file.
fn mp4dump(fragment: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!(
        "multimux-w13-mp4dump-{}-{}",
        std::process::id(),
        fragment.len()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("f.mp4");
    std::fs::write(&path, fragment).expect("write fragment");
    let out = std::process::Command::new("mp4dump")
        .args(["--verbosity", "2"])
        .arg(&path)
        .output()
        .expect("spawn mp4dump");
    let _ = std::fs::remove_dir_all(&dir);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The first unsigned integer on the line containing `marker`.
fn dumped_u64_after(text: &str, marker: &str) -> Option<u64> {
    let pos = text.find(marker)?;
    let line = text[pos..].lines().next()?;
    line.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .find_map(|s| s.parse::<u64>().ok())
}

/// Test 5: Config round-trip — `"smooth"` deserialises and validates.
#[test]
fn config_smooth_deserializes_and_validates() {
    let cfg: multimux::config::Config = serde_json::from_str(
        r#"{
            "bind": "0.0.0.0:8080",
            "routes": [
                {
                    "name": "test",
                    "input": {"type": "rtsp", "url": "rtsp://example.com/stream"},
                    "outputs": ["smooth"]
                }
            ]
        }"#,
    )
    .expect("deserialize smooth config");
    assert_eq!(cfg.routes.len(), 1);
    assert_eq!(cfg.routes[0].outputs.len(), 1, "one output configured");
    assert_eq!(
        cfg.routes[0].outputs[0].name(),
        "smooth",
        "output kind must be smooth"
    );
    cfg.validate().expect("smooth config must validate");
}
