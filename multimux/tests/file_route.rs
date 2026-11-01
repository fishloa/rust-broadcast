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
use multimux::serve_with_registry_on;

#[path = "support/listener.rs"]
mod listener;
use listener::bind_tcp;

/// Length of `h264_aac.ts` (video PTS 133200 -> 399600 at 90 kHz), and the
/// tolerance (one part) for segmenter boundary rounding.
const FIXTURE_SECS: f64 = 2.955;
const TAIL_TOLERANCE_SECS: f64 = 0.1;

fn fixture_path() -> String {
    test_fixtures::path("ts/h264_aac.ts")
        .to_string_lossy()
        .into_owned()
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
            ts_hls_cut: Default::default(),
            ts_hls_idr_fallback: Default::default(),
            declared_bandwidth_bps: None,
            audio_renditions: Vec::new(),
            audio_rendition_sources: Vec::new(),
            closed_caption_overrides: Vec::new(),
            audio_profiles: Vec::new(),
            subtitles: true,
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
    let (bind_addr, bind_listener) = bind_tcp();
    let config = file_config(bind_addr, fixture_path(), false);

    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));

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
    let (bind, bind_listener) = bind_tcp();
    let config = file_config(bind, fixture_path(), false);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    // Wait for the first pass's segments to be served, then let the single
    // pass fully drain (well past its ~3 s duration, paced near-realtime),
    // *then* snapshot the served set.
    poll_until_extinf(&client, &playlist_url).await;
    let drained = tokio::time::Instant::now() + Duration::from_secs(10);
    let body = loop {
        let body = client
            .get(&playlist_url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        if total_extinf_secs(&body) >= FIXTURE_SECS - TAIL_TOLERANCE_SECS
            || tokio::time::Instant::now() >= drained
        {
            break body;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
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
        served >= FIXTURE_SECS - TAIL_TOLERANCE_SECS,
        "loop:false must serve the file's tail: served {served:.3}s, expected ~2.955s \
         (a parked-but-healthy driver that skips the flush drops up to one target_duration)"
    );

    // The tail is flushed and the driver has ended; a settle window proves the
    // served set does not grow.
    tokio::time::sleep(Duration::from_millis(1500)).await;
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

/// `loop: true` on ONE route: keeps serving past the single pass (a NEW segment
/// URI appears that the first snapshot did not carry), paces near realtime (the
/// 3rd distinct segment lands >= 0.6 s in, not ~300x faster, and within 10 s),
/// and keeps pacing across the loop point (segments 6 -> 8, pass 2, span >=
/// 400 ms).
///
/// MUTATION VERIFIED, recorded verbatim: removing `FileIngestSession::refill`'s
/// `pass_wall_start` advance makes the loop-point assertion FAIL with:
///
///     segments past the loop point must stay at roughly realtime, not dump all of pass 2 at once: segments 6->8 in 41ns
///
/// The ~3 s fixture with a 0.5 s target makes ~6 segments per pass; the 300x
/// dump (the original bug) closes every segment ~instantly.
#[tokio::test]
async fn file_route_loop_true_keeps_serving_and_paces_across_the_loop_point() {
    let (bind, bind_listener) = bind_tcp();
    let config = file_config(bind, fixture_path(), true);
    let server = tokio::spawn(serve_with_registry_on(
        bind_listener,
        config,
        SchemeRegistry::new(),
    ));
    let client = reqwest::Client::new();
    let playlist_url = format!("http://{bind}/cam/media.m3u8");

    let first = poll_until_extinf(&client, &playlist_url).await;
    let t0 = segment_uris(&first);
    assert!(!t0.is_empty(), "the first pass must serve segments");
    let start = tokio::time::Instant::now();

    let mut seen: std::collections::HashSet<String> = t0.iter().cloned().collect();
    let mut at_3: Option<tokio::time::Instant> = None;
    let mut at_6: Option<tokio::time::Instant> = None;
    let mut at_8: Option<tokio::time::Instant> = None;
    let deadline = start + Duration::from_secs(20);
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
        let now = tokio::time::Instant::now();
        if seen.len() >= 3 && at_3.is_none() {
            at_3 = Some(now);
        }
        if seen.len() >= 6 && at_6.is_none() {
            at_6 = Some(now);
        }
        if seen.len() >= 8 && at_8.is_none() {
            at_8 = Some(now);
        }
        assert!(
            now < deadline,
            "loop:true must keep serving fresh segments past the first pass; only saw {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let fresh = seen.iter().filter(|u| !t0.contains(u)).count();
    assert!(
        fresh > 0,
        "loop:true must keep serving fresh segments past the first pass; only saw {t0:?}"
    );

    let to_3 = at_3.expect("3rd segment observed") - start;
    assert!(
        to_3 >= Duration::from_millis(600),
        "segments must appear at roughly realtime, not 300x -- 3 closed segments in {to_3:?}"
    );
    assert!(
        to_3 <= Duration::from_secs(10),
        "segment cadence must not stall -- 3 closed segments took {to_3:?}"
    );

    let span = at_8.expect("8th segment observed") - at_6.expect("6th segment observed");
    assert!(
        span >= Duration::from_millis(400),
        "segments past the loop point must stay at roughly realtime, not dump all of pass 2 at once: segments 6->8 in {span:?}"
    );

    server.abort();
}
