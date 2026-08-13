//! End-to-end coverage of the `InputSpec::File` route (issue #748 WP5): a
//! file route actually streams — real media bytes in through
//! `multimux::serve_with_registry`/`spawn_ingest`, real servable segments out
//! over a real HTTP `GET` of the resulting LL-HLS media playlist.
//!
//! This exists to prove the point of the whole work package: that a `File`
//! route is wired into `spawn_ingest` and reaches the **serving** layer — not
//! merely that its ingest lands samples in a Trunk, which would pass even if
//! `crate::source::advance_route` were never called (the exact failure mode
//! that call exists to prevent — a route that ingests but serves nothing).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use multimux::config::{Config, InputSpec, Route};
use multimux::dvr::DvrConfig;
use multimux::output::OutputKind;
use multimux::registry::SchemeRegistry;
use multimux::serve_with_registry;

fn fixture_path() -> String {
    format!("{}/../fixtures/ts/h264_aac.ts", env!("CARGO_MANIFEST_DIR"))
}

fn reserve_tcp_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve tcp port");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr
}

/// One LL-HLS-only route named `"cam"`, bound at `bind`, ingesting a
/// `File` route for `path` with `loop: false`. Short segments (0.5 s /
/// 100 ms) so the ~3 s real fixture closes several real segments.
fn file_config(bind: SocketAddr, path: String, loop_file: bool) -> Config {
    Config {
        bind: bind.to_string(),
        target_duration_secs: 0.5,
        part_target_ms: 100,
        window_segments: 8,
        routes: vec![Route {
            name: "cam".to_string(),
            input: InputSpec::File { path, loop_file },
            outputs: vec![OutputKind::LlHls],
            dvr: DvrConfig::default(),
        }],
        ..Config::default()
    }
}

/// Polls `playlist_url` until its body carries a real closed-segment
/// `#EXTINF:` line — never satisfied by `#EXT-X-MAP`/`#EXT-X-PART-INF`, so a
/// route with zero closed segments (the `advance_route`-removed mutation)
/// cannot pass. Generous hang guard, not a latency assertion.
async fn poll_until_extinf(client: &reqwest::Client, playlist_url: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(resp) = client.get(playlist_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && body.contains("#EXTINF:")
        {
            return body;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "no #EXTINF: line appeared in {playlist_url} within the hang guard -- \
                 file route never produced a servable closed segment"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A `File` route serves: after `serve_with_registry` spawns its ingest
/// (which drives the file through `advance_route`), the route's served LL-HLS
/// media playlist carries a real closed-segment `#EXTINF:` line — not merely
/// that samples landed in a Trunk.
///
/// MUTATION PROOF, recorded verbatim: removing the
/// `crate::source::advance_route(&driver, route_handle, &mut progress)` call
/// from `crate::source::file_reader::run_file_source` makes the file's samples
/// still land in the Trunk but never become servable segments — no segmenter
/// is pumped, so no closed segment is ever produced and this test FAILS with:
///
///     no #EXTINF: line appeared in http://127.0.0.1:<port>/cam/media.m3u8 within the hang guard -- file route never produced a servable closed segment
///
/// Restoring the call (and a `touch` of the restored files so cargo does not
/// serve a stale binary) makes it pass again.
#[tokio::test]
async fn file_route_serves_real_media_segments() {
    let bind_addr = reserve_tcp_addr();
    let config = file_config(bind_addr, fixture_path(), false);

    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));

    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind_addr}/cam/media.m3u8");
    let playlist = poll_until_extinf(&client, &playlist_url).await;

    assert!(
        playlist.contains("#EXTINF:"),
        "media playlist must carry a real closed-segment #EXTINF line: {playlist}"
    );

    // The init segment is also served — non-emptiness already asserted by the
    // helper, and it proves the route's segment/init store is genuinely wired.
    let init_url = format!("http://{bind_addr}/cam/init-1.mp4");
    let _init: bytes::Bytes = get_non_empty(&client, &init_url).await;

    server.abort();
}

/// Fetches `url`, asserting `200 OK` and a non-empty body.
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

/// Sanity: an unused-import guard that `InputSpec::File` stays constructible
/// through the public API (the whole feature — the other input kinds are
/// unrelated and not exercised here).
#[test]
fn file_is_the_feature() {
    let _ = InputSpec::File {
        path: "/a.ts".to_string(),
        loop_file: true,
    };
    let _arc: Arc<()> = Arc::new(());
    let _ = _arc;
}

/// Extract the `seg-*.m4s` URIs referenced by a rendered media playlist.
fn segment_uris(playlist: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = playlist;
    while let Some(i) = rest.find("seg-") {
        let seg = &rest[i..];
        let end = seg.find(".m4s").map(|e| e + ".m4s".len()).unwrap_or(0);
        if end > 0 {
            out.push(seg[..end].to_string());
            rest = &seg[end..];
        } else {
            rest = &seg[1..];
        }
    }
    out
}

/// `loop: true` keeps serving: new servable segments keep appearing past the
/// point the single pass of the file would have ended. The ~3 s fixture with a
/// 0.5 s target makes ~6 segments per pass; a bounded wait well beyond one
/// pass's duration must observe a NEW segment URI (the looped content's),
/// which a `loop: false` route would never produce.
#[tokio::test]
async fn file_route_loop_true_keeps_serving() {
    let bind = reserve_tcp_addr();
    let config = file_config(bind, fixture_path(), true);
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    // Wait for the first pass's segments, then snapshot the served set.
    poll_until_extinf(&client, &playlist_url).await;
    let t0 = segment_uris(
        &client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    );
    assert!(!t0.is_empty(), "the first pass must serve segments");

    // Wait (bounded) for a NEW segment URI to appear — a looped pass's.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let body = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let now = segment_uris(&body);
        let fresh = now.iter().filter(|u| !t0.contains(u)).count();
        if fresh > 0 {
            server.abort();
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "loop:true must keep serving fresh segments past the first pass; only saw {:?}",
            t0
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Sum the `#EXTINF:<seconds>,` durations of a rendered media playlist — the
/// total media duration the served segments cover, used to prove a finite file
/// serves its *entire* content (its tail segment included).
fn total_extinf_secs(playlist: &str) -> f64 {
    let mut total = 0.0;
    for line in playlist.lines() {
        if let Some(rest) = line.strip_prefix("#EXTINF:")
            && let Some(comma) = rest.find(',')
            && let Ok(secs) = rest[..comma].trim().parse::<f64>()
        {
            total += secs;
        }
    }
    total
}

/// `loop: false` stops: after producing its segments, the playlist stops
/// growing — the served set of segment URIs is unchanged across a bounded wait
/// well past the file's duration (a `loop: true` route, by contrast, would keep
/// adding new ones). Crucially, the finite file serves its **entire** content —
/// the final partial segment (the tail) must be flushed when the driver ends,
/// not silently dropped (which used to lose up to one `target_duration`).
#[tokio::test]
async fn file_route_loop_false_stops() {
    let bind = reserve_tcp_addr();
    let config = file_config(bind, fixture_path(), false);
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    // Wait for the first pass's segments to be served, then let the single
    // pass fully drain (well past its ~3 s duration, paced near-realtime),
    // *then* snapshot the served set.
    poll_until_extinf(&client, &playlist_url).await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    let body = client
        .get(&playlist_url)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let t0 = segment_uris(&body);
    assert!(
        !t0.is_empty(),
        "loop:false must serve the first pass's segments"
    );

    // The finite file must serve its WHOLE content: the total served media
    // duration has to reach the file's own ~2.955 s (video PTS 133200 → 399600
    // at 90 kHz), NOT stop ~one target_duration short of it (the un-flushed
    // tail). A generous tolerance (one part, 100 ms) keeps this robust to
    // segmenter boundary rounding while still catching the dropped tail.
    //
    // MUTATION VERIFIED, recorded verbatim: removing `run_file_source`'s
    // `driver.finish()` + final `advance_route` (parking without flushing)
    // makes this test FAIL with:
    //
    //     loop:false must serve the file's tail: served 2.000s, expected ~2.955s (a parked-but-healthy driver that skips the flush drops up to one target_duration)
    //
    // — the tail segment (~0.955 s, the partial final target_duration) is
    // silently dropped. Restoring the flush (and a `touch`) makes it pass
    // again.
    let served = total_extinf_secs(&body);
    assert!(
        served >= 2.955 - 0.1,
        "loop:false must serve the file's tail: served {served:.3}s, expected ~2.955s \
         (a parked-but-healthy driver that skips the flush drops up to one target_duration)"
    );

    // Let the file's single pass fully drain (well past its ~3 s duration), then
    // assert the served set has not grown.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let later = segment_uris(
        &client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    );
    let fresh = later.iter().filter(|u| !t0.contains(u)).count();
    assert_eq!(
        fresh,
        0,
        "loop:false must stop producing segments, but {:?} is new",
        later.iter().filter(|u| !t0.contains(u)).collect::<Vec<_>>()
    );
    server.abort();
}

/// `loop: true` paces to roughly realtime: a new closed segment appears every
/// ~`target_duration` (0.5 s), not ~300× faster (the bug dumped the whole
/// 3 s file every 10 ms tick). We assert only cadence — the wall time to
/// accumulate a handful of segments stays within a sane factor of realtime —
/// never exact counts, to stay robust to scheduler jitter.
#[tokio::test]
async fn file_route_loop_true_paces_near_realtime() {
    let bind = reserve_tcp_addr();
    let config = file_config(bind, fixture_path(), true);
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    // First closed segment appears.
    poll_until_extinf(&client, &playlist_url).await;
    let start = tokio::time::Instant::now();

    // Accumulate a handful of distinct segment URIs, measuring wall time. At
    // realtime (0.5 s target) the 3rd distinct segment lands ~1 s in; the bug's
    // 300× rate lands them ~instantly.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let want = 3usize;
    let deadline = start + Duration::from_secs(20);
    loop {
        let body = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        for u in segment_uris(&body) {
            seen.insert(u);
        }
        if seen.len() >= want {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "loop:true must keep producing segments; only saw {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let elapsed = start.elapsed();

    // Cadence: far slower than the 300× dump (which is ~milliseconds) and far
    // from a stall. At realtime, `want` distinct segments ≈ `want * 0.5 s` =
    // 1.5 s; a 0.6 s floor and 10 s ceiling give wide, deterministic bounds.
    assert!(
        elapsed >= Duration::from_millis(600),
        "segments must appear at roughly realtime, not 300× — {want} closed segments in {elapsed:?}"
    );
    assert!(
        elapsed <= Duration::from_secs(10),
        "segment cadence must not stall — {want} closed segments took {elapsed:?}"
    );

    server.abort();
}

/// Pacing survives the loop point (blocker 1's gap): the original cadence
/// test ends at the 3rd segment, entirely inside pass 1, so a baseline that
/// fails to advance past pass 1 (dumping the whole pass at ~300×) was never
/// observed. The ~3 s fixture with a 0.5 s target makes ~6 segments per pass;
/// this test measures the wall time between the 6th and 8th distinct segment
/// (the boundary crossing into pass 2) and asserts it stays at realtime —
/// a `pass_wall_start` that fails to advance would hand out all of pass 2 in
/// one drain and close those segments ~instantly.
///
/// MUTATION VERIFIED, recorded verbatim: removing `FileIngestSession::refill`'s
/// `pass_wall_start` advance makes this test FAIL with:
///
///     segments past the loop point must stay at roughly realtime, not dump all of pass 2 at once: segments 6→8 in 41ns
///
/// — pass 2 closes its segments in one drain (41 ns), the original ~300×
/// behaviour restored from pass 2 onward. Restoring the advance (and a `touch`)
/// makes it pass again.
#[tokio::test]
async fn file_route_loop_true_paces_past_first_pass() {
    let bind = reserve_tcp_addr();
    let config = file_config(bind, fixture_path(), true);
    let server = tokio::spawn(serve_with_registry(config, SchemeRegistry::new()));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    poll_until_extinf(&client, &playlist_url).await;

    // Collect distinct segment URIs, recording the wall time the 6th and 8th
    // appear (6 crosses the ~6-segment pass-1 boundary, 8 is firmly inside
    // pass 2).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut at_6: Option<tokio::time::Instant> = None;
    let mut at_8: Option<tokio::time::Instant> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while seen.len() < 8 {
        let body = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        for u in segment_uris(&body) {
            seen.insert(u);
        }
        if seen.len() >= 6 && at_6.is_none() {
            at_6 = Some(tokio::time::Instant::now());
        }
        if seen.len() >= 8 && at_8.is_none() {
            at_8 = Some(tokio::time::Instant::now());
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "must reach 8 distinct segments within the hang guard; only saw {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let at_6 = at_6.expect("6th segment observed");
    let at_8 = at_8.expect("8th segment observed");

    // Two pass-2 segments at realtime take ~1 s (2 × 0.5 s target); the
    // un-advanced baseline closes them ~instantly. A 400 ms floor is a wide,
    // deterministic lower bound that a pass-2 dump cannot meet.
    let span = at_8 - at_6;
    assert!(
        span >= Duration::from_millis(400),
        "segments past the loop point must stay at roughly realtime, not dump all of pass 2 at once: segments 6→8 in {span:?}"
    );

    server.abort();
}
