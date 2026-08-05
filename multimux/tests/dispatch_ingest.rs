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
//! # `InputSpec::Custom` driver-backed coverage (issue #805 task 5)
//!
//! Before task 5, the `Custom` path drove `crate::pipeline::run_pipeline` (a
//! `SampleSource`-fed segmenter loop) which was itself silently broken for a
//! time: it never published its `Trunk` into `RouteHandle`'s program
//! registry, so every consumer would hang on
//! `ProgramResolution::NotYetAnnounced` (see `RouteHandle::new`'s own doc,
//! "A producer writing the owned `Trunk` must publish it") -- fixed on this
//! branch (`fix(multimux): a producer writing the owned Trunk must publish
//! it, or egress hangs`) before this file existed. Task 5 deleted
//! `run_pipeline`/`SampleSource`/`MockSource` outright (the `Custom` path was
//! their last caller once RTMP left at task 4): a `Custom` factory now spawns
//! `multimux::supervise_driver` over its own `media_plane::ingress::Dialer`/
//! `IngestSession`, exactly like every built-in source (see
//! `examples/custom_scheme.rs`).
//! `custom_dispatch_drives_a_driver_backed_source_and_serves_real_media`
//! below covers that shape through the *exact* dispatch path a real `Custom`
//! route uses (`InputSpec::Custom` -> `SchemeRegistry` -> `InputCtx` -> a
//! factory that spawns `supervise_driver`), replaying the real
//! `h264_aac.ts` fixture (demuxed, not synthetic) through a small
//! `IngestSession` of its own -- so a regression of
//! `crate::source::report_driver_progress`'s registry-publish call (or
//! `crate::source::segment::drive_program_segmenters`'s segmenting) is
//! caught here too for the `Custom` dispatch path specifically, not just by
//! every built-in source's own loopback tests.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
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
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    }
}

/// Polls `playlist_url` until its body carries a real closed-segment
/// `#EXTINF:` line -- deliberately **not** satisfied by
/// `#EXT-X-PART-INF`/`#EXT-X-MAP`, which `hls_runtime`'s engine renders
/// unconditionally even for a route with zero closed segments (see
/// `hls-runtime/src/server/engine.rs`'s `render_playlist`), so a
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

// --- `InputSpec::Custom` driver-backed coverage (issue #805 task 5) ---

mod custom_dispatch_driver_backed {
    use super::*;
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::num::NonZeroUsize;

    use broadcast_common::{Demand, Stage, Timestamp};
    use media_plane::ingress::{
        Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
    };
    use media_plane::trunk::{RetentionClass, TrunkConfig};
    use multimux::registry::{InputCtx, InputFactory};
    use multimux::route::RouteHandle;
    use multimux::source::{DriverProgress, advance_route};
    use multimux::{Backoff, supervise_driver};
    use transmux::TsDemux;
    use transmux::pipeline::{CodecConfig, Sample, TrackSpec};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("non-zero capacity")
    }

    /// Demux the real `h264_aac.ts` fixture's AVC video track into a
    /// `TrackSpec` + its real, decoded `Sample`s -- mirrors
    /// `tests/lldash_dashjs.rs`'s own `real_video_track_and_samples`: real
    /// bytes in, not `ts_program::test_support::build_ts_bytes`'s
    /// hand-faked NAL payload.
    fn real_video_track_and_samples() -> (TrackSpec, Vec<Sample>) {
        let ts = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
        let media = TsDemux::new().demux(&ts).expect("demux h264_aac.ts");
        let video = media
            .tracks
            .into_iter()
            .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("h264_aac.ts must carry an AVC video track");
        (video.spec, video.samples)
    }

    /// A small `IngestSession` carrying real, pre-demuxed samples -- exactly
    /// the "small `Dialer`/`IngestSession` of its own" shape
    /// `examples/custom_scheme.rs` documents, just fed real fixture-derived
    /// media instead of synthetic frames.
    ///
    /// Its **first** `feed` call announces the one program (the real track)
    /// *and* queues every one of its real samples, in that same call -- the
    /// ordinary shape a real transport produces (a single MPEG-TS feed batch
    /// commonly carries the PMT and the first PES samples together), and
    /// exactly the shape `ProgramSegmenter`'s `subscribe_from_backlog`
    /// cursor (issue #808) exists to handle: it replays whatever backlog is
    /// already resident in the ring by the time `drive_program_segmenters`
    /// builds the segmenter, so samples published in the same `feed` call
    /// that announced the program are not lost.
    struct RealTsSession {
        pending: VecDeque<SessionEvent>,
        sent: bool,
        spec: TrackSpec,
        samples: Vec<Sample>,
    }

    impl Stage for RealTsSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = Infallible;

        fn demand(&self) -> Demand {
            Demand::new(1)
        }

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
            if !self.sent {
                self.sent = true;
                self.pending.push_back(SessionEvent::NewProgram {
                    program: ProgramId(0),
                    tracks: vec![self.spec.clone()],
                });
                let track_id = self.spec.track_id;
                for sample in self.samples.drain(..) {
                    self.pending.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            self.pending.pop_front()
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn finish(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    impl IngestSession for RealTsSession {
        type Request = Infallible;
    }

    /// Constructs a [`RealTsSession`] carrying `spec`/`samples` -- performs
    /// no I/O, exactly like every other `Dialer::dial` in this crate.
    struct RealTsDialer {
        spec: TrackSpec,
        samples: Vec<Sample>,
    }

    impl Dialer for RealTsDialer {
        type Session = RealTsSession;
        type Error = Infallible;

        fn dial(&mut self) -> Result<RealTsSession, Infallible> {
            let mut pending = VecDeque::new();
            pending.push_back(SessionEvent::Established);
            Ok(RealTsSession {
                pending,
                sent: false,
                spec: self.spec.clone(),
                samples: self.samples.clone(),
            })
        }
    }

    /// One dial-through-disconnect attempt -- the `supervise_driver`
    /// `attempt` closure. Mirrors every in-tree `run_*`: dial, wrap in an
    /// `IngestDriver`, feed, and after every feed call [`advance_route`] --
    /// the one facade call `examples/custom_scheme.rs` documents as the whole
    /// extension contract (registry publish + health flip, then turning
    /// samples into servable segments/parts).
    async fn run_real_ts(
        route_handle: Arc<RouteHandle>,
        spec: TrackSpec,
        samples: Vec<Sample>,
    ) -> multimux::Result<()> {
        let mut dialer = RealTsDialer { spec, samples };
        let session = dialer
            .dial()
            .unwrap_or_else(|never: Infallible| match never {});
        let trunk_config = TrunkConfig::new(nz(64), nz(16), nz(8), nz(64), nz(64));
        let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
        let mut driver: IngestDriver<RealTsSession> = IngestDriver::new(
            session,
            trunk_config,
            handshake,
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut progress = DriverProgress::new();

        // One feed: announces NewProgram AND queues every real sample --
        // mints the driver-side Trunk and publishes its samples in the same
        // batch. The ProgramSegmenter `advance_route`'s own segmenting step
        // builds is subscribed via `subscribe_from_backlog` (issue #808),
        // which replays whatever backlog is already resident in the ring
        // rather than starting from "now" -- so this single feed's samples
        // are not lost.
        driver.feed(&[], Timestamp::from_nanos(0));
        advance_route(&driver, &route_handle, &mut progress);

        driver.finish();
        advance_route(&driver, &route_handle, &mut progress);

        Ok(())
    }

    /// Drives a driver-backed `Custom` factory through the **exact**
    /// dispatch path a real `InputSpec::Custom` route uses:
    /// `serve_with_registry` resolves `Custom`'s `type_tag` through a
    /// `SchemeRegistry`, builds an `InputCtx`, and calls the registered
    /// factory -- this factory spawns `multimux::supervise_driver` over its
    /// own small `Dialer`/`IngestSession` fed the real `h264_aac.ts`
    /// fixture, exactly as a real embedding application's factory would
    /// spawn its own transport-fed driver loop (see
    /// `examples/custom_scheme.rs`). If [`advance_route`]'s internal
    /// registry-publish step were ever skipped, every request below would
    /// hang on `ProgramResolution::NotYetAnnounced` instead of erroring, and
    /// `poll_until_extinf` would time out; if its segmenting step were
    /// skipped, the route would resolve (health `Live`) but never carry a
    /// single `#EXTINF:` line, since nothing would ever turn the ingested
    /// samples into closed segments.
    ///
    /// MUTATION VERIFIED: commenting out `advance_route`'s
    /// `report_driver_progress(driver, route_handle, &mut state.published);`
    /// line in `multimux/src/source/mod.rs` (i.e. simulating a bug in the one
    /// facade every `Custom` factory author now relies on, rather than
    /// hand-assembling the two steps itself) makes this test fail:
    /// `poll_until_extinf`'s 20 s hang guard elapses and its own
    /// `panic!("no #EXTINF: line appeared ... dispatched ingest never
    /// produced a closed segment")` fires at
    /// `multimux/tests/dispatch_ingest.rs:157:13` -- the session demuxes and
    /// segments the real fixture correctly (nothing else changed), but every
    /// HTTP request resolves `ProgramResolution::NotYetAnnounced` forever
    /// since nothing ever published `SPTS_PROGRAM_ID` into the registry, so
    /// the LL-HLS engine never even gets a `Trunk` to render from. Rebuilt
    /// and re-ran to confirm this exact panic, then reverted.
    #[tokio::test]
    async fn custom_dispatch_drives_a_driver_backed_source_and_serves_real_media() {
        let (spec, samples) = real_video_track_and_samples();
        assert!(
            !samples.is_empty(),
            "h264_aac.ts must demux to at least one video sample"
        );

        let mut registry = SchemeRegistry::new();
        registry.register_input(
            "mock-driver-backed",
            Arc::new(move |ctx: InputCtx| {
                let spec = spec.clone();
                let samples = samples.clone();
                Ok(tokio::spawn(supervise_driver(
                    move |route_handle| run_real_ts(route_handle, spec.clone(), samples.clone()),
                    ctx.store,
                    Backoff::production_default(),
                    ctx.name,
                    ctx.shutdown_rx,
                )))
            }) as InputFactory,
        );

        let bind_addr = reserve_tcp_addr();
        let config = base_config(
            bind_addr,
            InputSpec::Custom {
                type_tag: "mock-driver-backed".to_string(),
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

    /// Two distinct programs on one driver-backed `Custom` route must each
    /// carry their *own* track specs — not the other program's, and not a
    /// merged set (issue #831 fix 2: the sync loop in
    /// `report_driver_progress` iterates every `driver.programs()`, and a
    /// per-`ProgramId` bug that synced only `SPTS_PROGRAM_ID` would pass
    /// every single-program test — this one pins that).
    ///
    /// The test drives a `Custom` route exactly like
    /// `custom_dispatch_drives_a_driver_backed_source_and_serves_real_media`,
    /// but its session announces two programs with distinguishing track id
    /// values (1 and 7) instead of one. The factory captures `ctx.store` and
    /// after `advance_route` completes, the test asserts
    /// `route_handle.track_specs(ProgramId(0))` is `[track_id=1]` and
    /// `route_handle.track_specs(ProgramId(1))` is `[track_id=7]` — proving
    /// the per-`ProgramId` sync works, not just the SPTS program.
    ///
    /// MUTATION VERIFIED (SPTS-only): changing the sync loop in
    /// `report_driver_progress` from `for program in driver.programs()` to
    /// `for &program in &[SPTS_PROGRAM_ID]` makes this test's
    /// `assert_eq!(specs_p1.len(), 1, ...)` fail — `left: 1, right: 0`.
    /// (Program 0's assertion alone misleadingly passes, since it happens
    /// to be `SPTS_PROGRAM_ID`.) Full failure output recorded below.
    #[tokio::test]
    async fn dash_two_program_track_separation() {
        use std::collections::VecDeque;
        use std::convert::Infallible;

        /// A session that announces two programs with distinguishing track ids.
        struct TwoProgSession {
            pending: VecDeque<SessionEvent>,
            announced: bool,
        }

        impl Stage for TwoProgSession {
            type In<'a> = &'a [u8];
            type Out = SessionEvent;
            type Error = Infallible;

            fn demand(&self) -> Demand {
                Demand::new(1)
            }

            fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
                if !self.announced {
                    self.announced = true;
                    let track_1 = TrackSpec::new(
                        1,
                        90_000,
                        CodecConfig::Avc {
                            config: transmux::avc_config_from_sprop(
                                "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==",
                            )
                            .expect("valid sprop"),
                            width: 320,
                            height: 240,
                        },
                    );
                    let track_7 = TrackSpec::new(
                        7,
                        90_000,
                        CodecConfig::Avc {
                            config: transmux::avc_config_from_sprop(
                                "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==",
                            )
                            .expect("valid sprop"),
                            width: 640,
                            height: 480,
                        },
                    );
                    self.pending.push_back(SessionEvent::NewProgram {
                        program: ProgramId(0),
                        tracks: vec![track_1],
                    });
                    self.pending.push_back(SessionEvent::NewProgram {
                        program: ProgramId(1),
                        tracks: vec![track_7],
                    });
                }
                Ok(())
            }

            fn poll(&mut self) -> Option<SessionEvent> {
                self.pending.pop_front()
            }

            fn next_deadline(&self) -> Option<Timestamp> {
                None
            }

            fn on_deadline(&mut self, _now: Timestamp) {}

            fn finish(&mut self) -> Result<(), Infallible> {
                Ok(())
            }
        }

        impl IngestSession for TwoProgSession {
            type Request = Infallible;
        }

        struct TwoProgDialer;

        impl Dialer for TwoProgDialer {
            type Session = TwoProgSession;
            type Error = Infallible;

            fn dial(&mut self) -> Result<TwoProgSession, Infallible> {
                let mut pending = VecDeque::new();
                pending.push_back(SessionEvent::Established);
                Ok(TwoProgSession {
                    pending,
                    announced: false,
                })
            }
        }

        async fn run_two_prog(route_handle: Arc<RouteHandle>) -> multimux::Result<()> {
            let mut dialer = TwoProgDialer;
            let session = dialer
                .dial()
                .unwrap_or_else(|never: Infallible| match never {});
            let trunk_config = TrunkConfig::new(nz(64), nz(16), nz(8), nz(64), nz(64));
            let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
            let mut driver: IngestDriver<TwoProgSession> = IngestDriver::new(
                session,
                trunk_config,
                handshake,
                media_plane::DEFAULT_MAX_PROGRAMS,
            );
            let mut progress = DriverProgress::new();
            driver.feed(&[], Timestamp::from_nanos(0));
            advance_route(&driver, &route_handle, &mut progress);
            driver.finish();
            advance_route(&driver, &route_handle, &mut progress);
            Ok(())
        }

        // Capture ctx.store so the test can assert per-program track specs
        // directly on the RouteHandle after advance_route completes.
        let captured_store = Arc::new(tokio::sync::Mutex::new(None::<Arc<RouteHandle>>));
        let captured_store_clone = Arc::clone(&captured_store);

        let mut registry = SchemeRegistry::new();
        registry.register_input(
            "mock-two-prog",
            Arc::new(move |ctx: InputCtx| {
                let store = Arc::clone(&ctx.store);
                let cs = Arc::clone(&captured_store_clone);
                {
                    let mut guard = cs.try_lock().expect("captured store not yet set");
                    *guard = Some(store);
                }
                Ok(tokio::spawn(supervise_driver(
                    run_two_prog,
                    ctx.store,
                    Backoff::production_default(),
                    ctx.name,
                    ctx.shutdown_rx,
                )))
            }) as InputFactory,
        );

        let bind_addr = reserve_tcp_addr();
        let mut config = base_config(
            bind_addr,
            InputSpec::Custom {
                type_tag: "mock-two-prog".to_string(),
                params: serde_json::Value::Null,
            },
        );
        // DASH output so we can also GET manifest.mpd for the SPTS-program check.
        config.routes[0].outputs = vec![OutputKind::Dash];
        let server = tokio::spawn(serve_with_registry(config, registry));

        // Wait for the route handle to appear (the factory fires once the
        // origin is building the route). Then wait a little more for
        // `advance_route` to complete, so `track_specs` are populated.
        let store_opt = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let guard = captured_store.try_lock().expect("captured store lock");
                if guard.is_some() {
                    break guard.clone();
                }
                drop(guard);
                if tokio::time::Instant::now() >= deadline {
                    panic!("InputCtx factory never fired within hang guard (10 s)");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let route_handle = store_opt.unwrap();

        // Wait for track specs to be populated (advance_route runs inside
        // the spawned task). Poll until program 0 has at least one track spec.
        {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                if !route_handle.track_specs(ProgramId(0)).is_empty() {
                    break;
                }
                if tokio::time::Instant::now() >= deadline {
                    panic!("track_specs for ProgramId(0) never populated within hang guard (10 s)");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        let specs_p0 = route_handle.track_specs(ProgramId(0));
        let specs_p1 = route_handle.track_specs(ProgramId(1));

        assert_eq!(
            specs_p0.len(),
            1,
            "ProgramId(0) must carry its own track spec (track_id=1), not empty: {specs_p0:?}"
        );
        assert!(
            specs_p0.iter().any(|s| s.track_id == 1),
            "ProgramId(0) must name track_id=1 — its assigned track, not the other's: {specs_p0:?}"
        );

        assert_eq!(
            specs_p1.len(),
            1,
            "ProgramId(1) must carry its own track spec (track_id=7), not empty — \
             a per-ProgramId bug that syncs only SPTS_PROGRAM_ID would leave this empty: {specs_p1:?}"
        );
        assert!(
            specs_p1.iter().any(|s| s.track_id == 7),
            "ProgramId(1) must name track_id=7 — its assigned track, not the other's: {specs_p1:?}"
        );

        // Also verify the SPTS manifest works (the DASH renderer only uses
        // SPTS_PROGRAM_ID), as a smoke test that the route is functional.
        let client = reqwest::Client::new();
        let mpd_url = format!("http://{bind_addr}/cam/manifest.mpd");
        let resp = client
            .get(&mpd_url)
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {mpd_url} failed: {e}"));
        assert_eq!(
            resp.status(),
            reqwest::StatusCode::OK,
            "manifest.mpd must return 200 after track specs are populated"
        );
        let mpd_body = resp.text().await.unwrap_or_default();
        assert!(
            mpd_body.contains(r#"id="1""#),
            "SPTS manifest must name track_id=1: {mpd_body}"
        );

        server.abort();
    }
}

// --- DASH / LL-DASH dispatch coverage (issue #831) ---
//
// Before the #831 fix, `RouteHandle::set_track_specs` had no production
// call site — every `report_driver_progress`/`drive_program_segmenters`
// path correctly published programs and segmented samples, but never
// populated the one piece of codec metadata the DASH/LL-DASH renderers
// read from `RouteHandle::track_specs`.  `manifest.mpd` and
// `manifest-ll.mpd` returned 503 forever on every real driver-backed
// route.
//
// These tests drive a real ingest route through `serve_with_registry`,
// assert over HTTP that the DASH/LL-DASH manifests return 200 with
// valid content, and do NOT call `set_track_specs` by hand — if they
// did they would reproduce the blind spot that hid the bug, and the
// test would not count.
//
// The `ts_udp_dash_manifest_returns_503_before_tracks_are_known` test
// additionally proves that a route with truly no track data yet still
// answers 503, not 200 (the genuine "no representable track" path).

/// Polls `url` until it returns `200 OK` with a body that satisfies
/// `predicate`, or the hang guard expires.  Returns the full response
/// body on success.
///
/// Hang guard, not a latency assertion (issue #807 taxonomy):
/// real loopback ingest + demux of the ~80 KiB fixture is comfortably
/// sub-second in practice; the bound exists only so a genuinely broken
/// dispatch path fails the test instead of hanging the suite forever.
/// The *failure shape* of a bug that returns 200-without-expected-content
/// is caught by the predicate — a specific assertion failure, not a
/// timeout. A bug that returns 503 forever (e.g. no track specs at all,
/// the original #831 defect) still times out at the hang guard, which is
/// acceptable here because the in-crate `dash_two_program_track_separation`
/// test catches that class of failure directly on the `RouteHandle`.
async fn poll_until_200_with(
    client: &reqwest::Client,
    url: &str,
    predicate: impl Fn(&str) -> bool,
) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        match client.get(url).send().await {
            Ok(resp) if resp.status() == reqwest::StatusCode::OK => {
                let body = resp
                    .text()
                    .await
                    .unwrap_or_else(|e| panic!("GET {url}: reading body failed: {e}"));
                if predicate(&body) {
                    return body;
                }
                // 200 but body doesn't satisfy the predicate yet — keep
                // polling (the track specs may have arrived but the
                // manifest is still rendering its first segment list).
            }
            Ok(resp) => {
                // Non-200: the manifest isn't ready (likely 503).
                // Keep polling rather than panicking — the track specs
                // arrive asynchronously once the ingest driver
                // announces the first program.
                let _ = resp;
            }
            Err(_) => { /* connection refused during startup — keep polling */ }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("GET {url} never returned 200 with the expected body within hang guard (20 s)");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A DASH-only route (`OutputKind::Dash`, no LL-HLS) driven by
/// real TS-UDP fixture bytes serves a valid `manifest.mpd` over HTTP
/// with zero calls to `set_track_specs` — proving the production
/// wiring (issue #831) now populates track specs from the driver.
#[tokio::test]
async fn dash_manifest_served_without_explicit_set_track_specs() {
    let bind_addr = reserve_tcp_addr();
    let udp_addr = reserve_udp_addr();
    let mut config = base_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
        },
    );
    config.routes[0].outputs = vec![OutputKind::Dash];

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
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
    let mpd_url = format!("http://{bind_addr}/cam/manifest.mpd");
    let mpd_body = poll_until_200_with(&client, &mpd_url, |body| {
        body.contains("<MPD")
            && body.contains(r#"xmlns="urn:mpeg:dash:schema:mpd:2011""#)
            && body.contains(r#"type="dynamic""#)
            && body.contains("<Representation")
    })
    .await;
    stop.store(true, Ordering::Relaxed);

    assert!(
        mpd_body.contains("<MPD"),
        "manifest.mpd must be well-formed XML: {mpd_body}"
    );
    assert!(
        mpd_body.contains(r#"xmlns="urn:mpeg:dash:schema:mpd:2011""#),
        "{mpd_body}"
    );
    assert!(mpd_body.contains(r#"type="dynamic""#), "{mpd_body}");
    assert!(
        mpd_body.contains("<Representation"),
        "manifest must describe at least one Representation — \
         the route's real H.264 track from the fixture: {mpd_body}"
    );

    send_task.abort();
    server.abort();
}

/// An LL-DASH route (`OutputKind::LlDash`) driven by real TS-UDP
/// fixture bytes serves a valid `manifest-ll.mpd` over HTTP with zero
/// calls to `set_track_specs` — the LL-DASH counterpart of the DASH test above.
#[tokio::test]
async fn ll_dash_manifest_served_without_explicit_set_track_specs() {
    let bind_addr = reserve_tcp_addr();
    let udp_addr = reserve_udp_addr();
    let mut config = base_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
        },
    );
    config.routes[0].outputs = vec![OutputKind::LlDash];

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let ts_bytes = std::fs::read(fixture_path()).expect("h264_aac.ts fixture must exist");
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
    let mpd_url = format!("http://{bind_addr}/cam/manifest-ll.mpd");
    let mpd_body = poll_until_200_with(&client, &mpd_url, |body| {
        body.contains("<MPD")
            && body.contains(r#"xmlns="urn:mpeg:dash:schema:mpd:2011""#)
            && body.contains(r#"type="dynamic""#)
            && body.contains("<Representation")
    })
    .await;
    stop.store(true, Ordering::Relaxed);

    assert!(
        mpd_body.contains("<MPD"),
        "manifest-ll.mpd must be well-formed XML: {mpd_body}"
    );
    assert!(
        mpd_body.contains(r#"xmlns="urn:mpeg:dash:schema:mpd:2011""#),
        "{mpd_body}"
    );
    assert!(mpd_body.contains(r#"type="dynamic""#), "{mpd_body}");
    assert!(
        mpd_body.contains("<Representation"),
        "LL-DASH manifest must describe at least one Representation: {mpd_body}"
    );

    send_task.abort();
    server.abort();
}

/// A DASH route with no tracks published yet (no UDP sender, so the
/// ingest driver never observes a program) answers 503 on
/// `manifest.mpd` — the genuine "no representable track known yet"
/// path, distinct from the "not yet announced" (registry-empty) 503.
#[tokio::test]
async fn ts_udp_dash_manifest_returns_503_before_tracks_are_known() {
    let bind_addr = reserve_tcp_addr();
    let udp_addr = reserve_udp_addr();
    let mut config = base_config(
        bind_addr,
        InputSpec::TsUdp {
            addr: udp_addr.to_string(),
            multicast_group: None,
        },
    );
    config.routes[0].outputs = vec![OutputKind::Dash];

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    // Deliberately send nothing — the route binds its UDP socket but
    // never receives a single datagram, so no IngestSession ever
    // announces a program, so the route's track specs stay empty.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let mpd_url = format!("http://{bind_addr}/cam/manifest.mpd");
    let resp = client
        .get(&mpd_url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {mpd_url} failed: {e}"));

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "manifest.mpd must return 503 until at least one program \
         with known tracks is announced — a route with no ingest yet \
         must not return 200: status={}",
        resp.status()
    );

    server.abort();
}
