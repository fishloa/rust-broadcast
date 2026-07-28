//! Issue #805 task 3: end-to-end coverage of the *route-dispatch* path
//! itself -- `multimux::serve_with_registry` / `spawn_ingest` -- rather than
//! any one source driven directly.
//!
//! # The blind spot this file closes
//!
//! Before task 3, no test in this workspace entered through
//! `serve_with_registry`/`spawn_ingest` with real bytes flowing all the way
//! to a served HTTP response: `multimux/tests/rtsp_ingest.rs` drives
//! `RtspDialer`/`RtspIngestSession` directly, and `origin_llhls.rs`/
//! `lldash_dashjs.rs` build a `RouteHandle` and feed it by hand, never
//! referencing `InputSpec` or `serve_with_registry` at all. That is exactly
//! the shape of gap that let eight of nine `InputSpec` variants dispatch to
//! a stubbed no-op arm for a long time while every other gate (build/
//! clippy/doc, thousands of passing tests) stayed green -- see
//! `multimux/src/origin/mod.rs`'s own
//! `every_input_spec_variant_dispatches_to_real_ingest_not_a_stub` (the
//! cheap, exhaustive, in-crate regression net for *that*) for the full
//! story. This file is the deeper, representative half: real bytes in
//! through `serve_with_registry`, real media out over a real HTTP `GET`.
//!
//! # Real fixture, not synthetic bytes
//!
//! Streams the workspace's real, ffmpeg-encoded `fixtures/ts/h264_aac.ts`
//! capture (320x240 Main-profile H.264 @ 25 fps + AAC, ~3.0 s / 75 real
//! video frames with keyframes roughly every second -- see
//! `fixtures/ts/CODEC-ORACLE.md`) verbatim over the wire, exactly as a real
//! camera/HTTP origin would -- not `ts_program::test_support::build_ts_bytes`
//! (a muxed-but-hand-faked NAL payload used by this crate's own unit tests).
//!
//! # Driver-backed kinds
//!
//! `TsUdp` (a UDP socket) and `TsHttp` (a small loopback HTTP server) are the
//! cheapest of the nine to drive with a real fixture -- no RTSP/SRT
//! handshake, no out-of-band SDP, no HLS/DASH/Smooth manifest to author.
//! `Rtmp` (issue #805 task 4) is covered separately below with its own real
//! ffmpeg-captured publish, since it needs the RTMP handshake/`publish`
//! dance rather than a bare byte stream.
//!
//! # `run_pipeline` coverage
//!
//! `crate::pipeline::run_pipeline` (the `Custom` path, `Rtmp` having left it
//! at issue #805 task 4) was itself silently broken for a time: it never
//! published its `Trunk` into
//! `RouteHandle`'s program registry, so every consumer would hang on
//! `ProgramResolution::NotYetAnnounced` (see `RouteHandle::new`'s own doc,
//! "A producer writing the owned `Trunk` must publish it") -- fixed on this
//! branch (`fix(multimux): a producer writing the owned Trunk must publish
//! it, or egress hangs`) before this file existed.
//! `custom_dispatch_drives_run_pipeline_and_serves_real_media` below drives
//! `run_pipeline` through the *exact* dispatch path a real `Custom` route
//! uses (`InputSpec::Custom` -> `SchemeRegistry` -> `InputCtx` -> a factory
//! that spawns `run_pipeline`), so a regression of that publish call is
//! caught here too, not just by `pipeline.rs`'s own unit test (which never
//! goes through `serve_with_registry`/a real HTTP response).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry;

fn fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/h264_aac.ts"
    ))
}

/// The real ffmpeg-captured RTMP publish (`app=live`, `stream_key=testkey`,
/// H.264+AAC) -- see `tests/fixtures/PROVENANCE.md`.
fn rtmp_fixture_path() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/rtmp-obs-publish.bin"
    ))
}

/// Reserves a free TCP port, then immediately releases it -- the same
/// "reserve then drop, hand the exact address to the thing that binds it"
/// pattern `multimux/src/source/ts_udp.rs`'s own loopback test uses, just
/// for TCP (this crate's HTTP origin bind address).
fn reserve_tcp_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

/// Same as [`reserve_tcp_addr`], for a UDP port (the `TsUdp` route's own
/// bind address).
fn reserve_udp_addr() -> SocketAddr {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("reserve udp port");
    let addr = socket.local_addr().expect("local addr");
    drop(socket);
    addr
}

/// One LL-HLS-only route named `"cam"`, bound at `bind`, ingesting `input` --
/// short segments/parts (0.5 s / 100 ms) so the ~3 s real fixture closes at
/// least one real segment (two keyframes comfortably land inside the fed
/// data) well within this file's polling hang guards.
fn base_config(bind: SocketAddr, input: InputSpec) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input,
            outputs: vec![OutputKind::LlHls],
        }],
        ..Config::default()
    }
}

/// Polls `playlist_url` until its body carries a real closed-segment
/// `#EXTINF:` line -- deliberately **not** satisfied by
/// `#EXT-X-PART-INF`/`#EXT-X-MAP`, which `ll_hls_runtime`'s engine renders
/// unconditionally even for a route with zero closed segments (see
/// `ll-hls-runtime/src/server/engine.rs`'s `render_playlist`), so a
/// zero-segment route cannot accidentally pass this check.
///
/// A generous hang guard, not a latency assertion (issue #807): real
/// loopback ingest + demux + segmentation of this ~80 KiB fixture is
/// comfortably faster than this bound in practice; the bound exists only so
/// a genuinely broken/dead dispatch path fails the test instead of hanging
/// the suite forever.
async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.text().await {
                    if body.contains("#EXTINF:") {
                        return body;
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no #EXTINF: line appeared in {playlist_url} within the hang guard -- \
                 dispatched ingest never produced a closed segment"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Extracts the first `seg-{track}-{seq}.m4s` URI out of a rendered media
/// playlist body (mirrors `multimux/tests/origin_llhls.rs`'s own helper).
fn first_segment_uri(playlist: &str) -> &str {
    let start = playlist
        .find("seg-")
        .unwrap_or_else(|| panic!("no seg-*.m4s URI in playlist: {playlist}"));
    let rest = &playlist[start..];
    let end = rest
        .find(".m4s")
        .unwrap_or_else(|| panic!("no .m4s in playlist: {playlist}"))
        + ".m4s".len();
    &rest[..end]
}

/// Fetches `url`, asserting `200 OK` and a non-empty body -- returns the
/// bytes so a caller can assert further (e.g. structural conformance).
async fn get_non_empty(client: &reqwest::Client, url: &str) -> bytes::Bytes {
    let resp = client
        .get(url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {url} failed: {e}"));
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "GET {url}");
    let body = resp
        .bytes()
        .await
        .unwrap_or_else(|e| panic!("reading body of {url} failed: {e}"));
    assert!(!body.is_empty(), "GET {url}: body must be non-empty");
    body
}

/// Real fixture bytes in through `InputSpec::TsUdp` (a real UDP socket
/// `serve_with_registry` binds), real media out over a real HTTP `GET` of
/// the resulting LL-HLS media playlist/init/segment.
///
/// MUTATION VERIFIED: reverting `spawn_ingest`'s `InputSpec::TsUdp` arm (in
/// `multimux/src/origin/mod.rs`) to the pre-#805 combined stub (`{
/// tokio::spawn(async move { tracing::error!(..); }) }`) makes this test
/// fail: `poll_until_extinf` never sees `#EXTINF:` within its 20 s hang
/// guard (the stub never binds a socket, let alone ingests anything), and
/// the `panic!("no #EXTINF: line appeared ... dispatched ingest never
/// produced a closed segment")` inside it fires. Confirmed by applying the
/// same mutation this file's sibling regression net in
/// `multimux/src/origin/mod.rs` already mutation-verifies, rebuilding, and
/// re-running this test to see that exact panic; reverted afterwards.
#[tokio::test]
async fn ts_udp_dispatch_serves_real_media_end_to_end() {
    let bind_addr = reserve_tcp_addr();
    let udp_addr = reserve_udp_addr();
    let config = base_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
        },
    );

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
    // Resend the whole fixture on a loop (rather than once) so the (async,
    // therefore not synchronized with this test) socket bind inside
    // `serve_with_registry`'s spawned ingest task has ample opportunity to
    // land before a datagram that matters arrives -- exactly the pattern
    // `multimux/src/origin/mod.rs`'s own
    // `ts_udp_input_ingests_and_becomes_resolvable_through_the_registry` test
    // uses, just with real fixture bytes instead of
    // `ts_program::test_support::build_ts_bytes`.
    let stop = Arc::new(AtomicBool::new(false));
    let sender_stop = Arc::clone(&stop);
    let sender = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind sender");
    let send_task = tokio::spawn(async move {
        while !sender_stop.load(Ordering::Relaxed) {
            for chunk in ts_bytes.chunks(7 * 188) {
                let _ = sender.send_to(chunk, udp_addr).await;
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    });

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;
    stop.store(true, Ordering::Relaxed);

    assert!(
        playlist.contains("#EXTINF:"),
        "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
    );

    let init_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/init-1.mp4")).await;
    let _ = init_bytes; // non-emptiness already asserted by get_non_empty

    let seg_uri = first_segment_uri(&playlist).to_string();
    let _seg_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/{seg_uri}")).await;

    send_task.abort();
    server.abort();
}

/// Same property as [`ts_udp_dispatch_serves_real_media_end_to_end`], for
/// `InputSpec::TsHttp` (a small loopback HTTP server streaming the fixture
/// over chunked transfer-encoding, mirroring
/// `multimux/src/source/ts_http.rs`'s own
/// `start_chunked_ts_server`/`loopback_http_ts_yields_samples_after_pmt_resolves`
/// test) instead of a UDP socket.
///
/// MUTATION VERIFIED: reverting `spawn_ingest`'s `InputSpec::TsHttp` arm to
/// the pre-#805 combined stub (`{ tokio::spawn(async move {
/// tracing::error!(..); }) }`) makes this test fail: the exact same
/// `poll_until_extinf` 20 s hang-guard panic as
/// `ts_udp_dispatch_serves_real_media_end_to_end`'s own mutation-verify
/// above -- `"no #EXTINF: line appeared in http://127.0.0.1:<port>/cam/media.m3u8
/// within the hang guard -- dispatched ingest never produced a closed
/// segment"` (the stub never opens the GET at all). Rebuilt and re-ran to
/// confirm this exact panic, then reverted.
#[tokio::test]
async fn ts_http_dispatch_serves_real_media_end_to_end() {
    use axum::Router;
    use axum::body::Body;
    use axum::response::IntoResponse;
    use axum::routing::get;

    let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");

    async fn handler(body: axum::extract::State<Vec<u8>>) -> axum::response::Response {
        let chunks: Vec<std::result::Result<Vec<u8>, std::io::Error>> =
            body.0.chunks(7 * 188).map(|c| Ok(c.to_vec())).collect();
        let stream = futures_util::stream::iter(chunks);
        let body = Body::from_stream(stream);
        ([(axum::http::header::CONTENT_TYPE, "video/mp2t")], body).into_response()
    }
    let app = Router::new()
        .route("/stream.ts", get(handler))
        .with_state(ts_bytes);
    let ts_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port for the ts source");
    let ts_addr = ts_listener.local_addr().expect("local addr");
    let ts_server = tokio::spawn(async move {
        axum::serve(ts_listener, app).await.expect("axum ts server");
    });

    let bind_addr = reserve_tcp_addr();
    let config = base_config(
        bind_addr,
        InputSpec::TsHttp {
            url: format!("http://{ts_addr}/stream.ts"),
            auth: None,
        },
    );
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;

    assert!(
        playlist.contains("#EXTINF:"),
        "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
    );

    let _init_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/init-1.mp4")).await;
    let seg_uri = first_segment_uri(&playlist).to_string();
    let _seg_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/{seg_uri}")).await;

    server.abort();
    ts_server.abort();
}

/// Real fixture bytes in through `InputSpec::Rtmp` (issue #805 task 4 -- a
/// real TCP client playing back the captured ffmpeg publish against the
/// listen socket `serve_with_registry` binds), real media out over a real
/// HTTP `GET` of the resulting LL-HLS media playlist/init/segment -- the
/// same property as `ts_udp_dispatch_serves_real_media_end_to_end`, for the
/// one push-based (`Listener`-backed) input kind.
///
/// MUTATION VERIFIED: stubbing `spawn_ingest`'s `InputSpec::Rtmp` arm (in
/// `multimux/src/origin/mod.rs`) to the same dead-arm shape the `ts_udp`/
/// `ts_http` siblings' own mutation-verify uses (`tokio::spawn(async move {
/// tracing::error!(..); })`, never binding a listen socket) makes this test
/// fail exactly the same way: `poll_until_extinf`'s `panic!("no #EXTINF:
/// line appeared in http://127.0.0.1:<port>/cam/media.m3u8 within the hang
/// guard -- dispatched ingest never produced a closed segment")` fires after
/// its full 20 s hang guard elapses (confirmed: rebuilt with the stub in
/// place, ran `cargo test -p multimux --test dispatch_ingest
/// rtmp_dispatch_serves_real_media_end_to_end`, saw that exact panic at
/// `multimux/tests/dispatch_ingest.rs:141`, then reverted). This test proves
/// the *dispatch wiring* (`InputSpec::Rtmp` -> `run_rtmp` -> a real HTTP
/// response); `multimux/src/source/rtmp.rs`'s own tests separately
/// mutation-verify the concurrency fix and the first-sample-not-dropped
/// invariant *inside* `run_rtmp`/`RtmpIngestSession`, which this dispatch
/// test does not re-derive (RTMP already served media before issue #805
/// task 4 too -- this test's contribution is pinning the dispatch arm, not
/// distinguishing old from new architecture).
#[tokio::test]
async fn rtmp_dispatch_serves_real_media_end_to_end() {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    let bind_addr = reserve_tcp_addr();
    let rtmp_addr = reserve_tcp_addr();
    let config = base_config(
        bind_addr,
        InputSpec::Rtmp {
            listen: rtmp_addr.to_string(),
            app: None,
            stream_key: None,
        },
    );
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let fixture = std::fs::read(rtmp_fixture_path()).expect("rtmp fixture must exist");
    let publisher = tokio::spawn(async move {
        let mut stream = None;
        for _ in 0..200 {
            match TcpStream::connect(rtmp_addr).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        let mut stream = stream.expect("connect to the RTMP listener");
        stream
            .write_all(&fixture)
            .await
            .expect("write rtmp publish bytes");
        let mut sink = [0u8; 8192];
        loop {
            match stream.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    });

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;

    assert!(
        playlist.contains("#EXTINF:"),
        "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
    );

    let _init_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/init-1.mp4")).await;
    let seg_uri = first_segment_uri(&playlist).to_string();
    let _seg_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/{seg_uri}")).await;

    publisher.abort();
    server.abort();
}

// --- `run_pipeline` coverage (issue #805 task 3's "also cover
// `pipeline::run_pipeline`") -- gated on `testsupport` since it needs
// `multimux::pipeline::MockSource` ---

#[cfg(feature = "testsupport")]
mod run_pipeline_coverage {
    use super::*;
    use multimux::pipeline::{MockSource, run_pipeline};
    use multimux::registry::{InputCtx, InputFactory};
    use transmux::avc_config_from_sprop;
    use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    const VIDEO_TIMESCALE: u32 = 90_000;
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;

    fn video_track_spec() -> TrackSpec {
        let config = avc_config_from_sprop(SPROP).expect("valid sprop");
        TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        )
    }

    /// 90 frames @ 30 fps = 3 s, sync every 30 frames -- comfortably over
    /// `base_config`'s 0.5 s target duration, so at least one real segment
    /// closes (mirrors `multimux::pipeline`'s own
    /// `drives_source_through_segmenter_into_store` unit test's batch
    /// shape).
    fn batches() -> Vec<Vec<(u32, Sample)>> {
        (0..90u32)
            .map(|i| {
                let is_sync = i % 30 == 0;
                let data = vec![0xAAu8.wrapping_add((i % 251) as u8); 64];
                let sample = Sample::new(
                    data,
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(i64::from(i) * i64::from(FRAME_DUR)),
                    Some(FRAME_DUR),
                    is_sync,
                );
                vec![(1u32, sample)]
            })
            .collect()
    }

    /// Drives `pipeline::run_pipeline` through the **exact** dispatch path a
    /// real `InputSpec::Custom` route uses: `serve_with_registry` resolves
    /// `Custom`'s `type_tag` through a `SchemeRegistry`, builds an
    /// `InputCtx`, and calls the registered factory -- this factory spawns
    /// `run_pipeline` fed by a `MockSource`, exactly as a real embedding
    /// application's factory would spawn its own connector-fed
    /// `run_pipeline`/`supervise`. If `run_pipeline` ever again forgets to
    /// call `RouteHandle::publish_owned_trunk`, every request below hangs on
    /// `ProgramResolution::NotYetAnnounced` instead of erroring, and
    /// `poll_until_extinf` times out.
    ///
    /// MUTATION VERIFIED: removing `run_pipeline`'s
    /// `route_handle.publish_owned_trunk();` call (in
    /// `multimux/src/pipeline.rs`) makes this test fail:
    /// `poll_until_extinf`'s 20 s hang guard elapses and its own
    /// `panic!("no #EXTINF: line appeared ... dispatched ingest never
    /// produced a closed segment")` fires -- the route ingests and segments
    /// correctly (nothing else changed), but every HTTP request resolves
    /// `ProgramResolution::NotYetAnnounced` forever since nothing ever
    /// published `SPTS_PROGRAM_ID` into the registry, so the LL-HLS engine
    /// never even gets a `Trunk` to render from. Rebuilt and re-ran to
    /// confirm this exact panic, then reverted.
    #[tokio::test]
    async fn custom_dispatch_drives_run_pipeline_and_serves_real_media() {
        let mut registry = SchemeRegistry::new();
        registry.register_input(
            "mock-pipeline",
            Arc::new(|ctx: InputCtx| {
                Ok(tokio::spawn(async move {
                    let source = MockSource::new(vec![video_track_spec()], batches());
                    let _ = run_pipeline(
                        ctx.store,
                        ctx.target_duration_secs,
                        ctx.part_target_ms,
                        source,
                        &ctx.name,
                    )
                    .await;
                }))
            }) as InputFactory,
        );

        let bind_addr = reserve_tcp_addr();
        let config = base_config(
            bind_addr,
            InputSpec::Custom {
                type_tag: "mock-pipeline".to_string(),
                params: serde_json::Value::Null,
            },
        );
        let server = tokio::spawn(serve_with_registry(config, registry));

        let client = reqwest::Client::new();
        let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
        let playlist = poll_until_extinf(&client, &playlist_url).await;

        assert!(
            playlist.contains("#EXTINF:"),
            "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
        );

        let _init_bytes =
            get_non_empty(&client, &format!("http://{bind_addr}/cam/init-1.mp4")).await;
        let seg_uri = first_segment_uri(&playlist).to_string();
        let _seg_bytes = get_non_empty(&client, &format!("http://{bind_addr}/cam/{seg_uri}")).await;

        server.abort();
    }
}
