//! Audit 23 finding 230: the DASH-IF ingest handler's body/prefix limits,
//! the pump's running-health guard and the `Segment` object's number/time
//! mapping each have an observable test.
//!
//! Every test drives the real `run_dashif_ingest` listener over a loopback
//! TCP socket and reads the real trunk the pump publishes into; nothing here
//! inspects a mock of the handler.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
use multimux::route::{RouteConfig, RouteHandle};
use multimux::source::dashif_ingest::{DashifIngestRoute, run_dashif_ingest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use transmux::FragmentTrackData;
use transmux::dash::DashPackager;
use transmux::pipeline::{build_init_segment, build_media_segment};

/// Every blocking wait below is bounded.
const GUARD: Duration = Duration::from_secs(10);
/// Samples carried by each representation's one test segment.
const REP_SAMPLES: usize = 2;
/// How long a bounded drain waits for one more sample before concluding none
/// is coming.
const DRAIN_QUIET: Duration = Duration::from_millis(300);
/// A refused upload's body, several socket buffers deep so it is still in
/// flight when the route has already refused the request.
const REFUSED_BODY_BYTES: usize = 3 * 1024 * 1024;
/// How much of that body one write carries, so the server sees it arriving.
const REFUSED_BODY_CHUNK: usize = 64 * 1024;
/// Half-closing requests the regression test sends: enough that an HTTP/1
/// connection reading once more before it writes the answer is reliably
/// exercised.
const HALF_CLOSE_ROUNDS: usize = 64;

struct Rep {
    id: String,
    init: Vec<u8>,
    segment: Vec<u8>,
    samples: usize,
}

struct Feed {
    mpd: String,
    reps: Vec<Rep>,
}

/// A real MPD, init segment and one media segment per track, all built from
/// the committed `h264-aac.ts` fixture through the same packager a publisher
/// would use.
fn feed() -> Feed {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let media = test_harness::collect::media_from_ts(&ts[..]).expect("demux fixture");
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mpd = DashPackager::default()
        .render(&specs, &test_harness::collect::segment_log(&media))
        .expect("mpd");
    let mut reps = Vec::new();
    for (index, track) in media.tracks.iter().enumerate() {
        let init = build_init_segment(std::slice::from_ref(&track.spec), media.movie_timescale)
            .expect("init segment");
        let take = track.samples.len().min(REP_SAMPLES);
        let fragment = FragmentTrackData::new(
            track.spec.track_id,
            track.start_decode_time,
            &track.samples[..take],
        );
        let segment = build_media_segment(1, &[fragment]).expect("media segment");
        reps.push(Rep {
            id: (index + 1).to_string(),
            init,
            segment,
            samples: take,
        });
    }
    Feed { mpd, reps }
}

struct Rig {
    addr: SocketAddr,
    handle: Arc<RouteHandle>,
    task: tokio::task::JoinHandle<multimux::MultimuxError>,
}

/// Start the real ingest listener over a pre-bound loopback socket. The
/// publishing point is `/live`.
async fn start_with(
    configure: impl FnOnce(DashifIngestRoute) -> DashifIngestRoute,
    handshake: HandshakePolicy,
) -> Rig {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let route = configure(DashifIngestRoute::with_listener("cam", listener, "/live"));
    let handle = Arc::new(RouteHandle::new(RouteConfig::default()));
    let route_handle = Arc::clone(&handle);
    let task = tokio::spawn(async move {
        run_dashif_ingest(&route, TrunkConfig::default(), handshake, &route_handle).await
    });
    Rig { addr, handle, task }
}

fn never_times_out() -> HandshakePolicy {
    HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
}

fn http_request(method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

/// A request that declares one length but sends no body, so the handler's
/// own pre-body length check is the only thing that can refuse it.
fn declared_request(method: &str, path: &str, declared: usize) -> Vec<u8> {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nContent-Length: {declared}\r\n\
         Connection: close\r\n\r\n"
    )
    .into_bytes()
}

fn chunked_request(method: &str, path: &str, frames: &[&[u8]]) -> Vec<u8> {
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    for frame in frames {
        request.extend_from_slice(format!("{:X}\r\n", frame.len()).as_bytes());
        request.extend_from_slice(frame);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"0\r\n\r\n");
    request
}

/// The status code carried by one raw HTTP/1.1 response, if it carries one.
fn status_of(response: &[u8]) -> Option<u16> {
    String::from_utf8_lossy(response)
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
}

/// Read the whole response, stating which bytes arrived when the read fails:
/// a reset after a refusal must be reported as the reset it is, never as the
/// empty response it leaves behind.
async fn read_response(socket: &mut TcpStream, response: &mut Vec<u8>) {
    let read = tokio::time::timeout(GUARD, socket.read_to_end(response))
        .await
        .expect("read must not hang");
    if let Err(e) = read {
        panic!(
            "response read failed ({e:?}) after {} bytes: {:?}",
            response.len(),
            String::from_utf8_lossy(response)
        );
    }
}

/// Send one raw request, half-close the write side as a client that is done
/// sending, and return the response status code.
async fn status(addr: SocketAddr, request: Vec<u8>) -> u16 {
    let mut socket = tokio::time::timeout(GUARD, TcpStream::connect(addr))
        .await
        .expect("connect must not hang")
        .expect("connect");
    socket.write_all(&request).await.expect("write request");
    let _ = socket.shutdown().await;
    let mut response = Vec::new();
    read_response(&mut socket, &mut response).await;
    status_of(&response).expect("response status code")
}

async fn post(rig: &Rig, path: &str, body: &[u8]) -> u16 {
    status(rig.addr, http_request("POST", path, body)).await
}

/// Drain every sample currently offered, waiting `DRAIN_QUIET` for one more
/// once the cursor runs dry. Always bounded.
async fn drain_samples(cursor: &mut SampleCursor) -> usize {
    let mut count = 0;
    let deadline = tokio::time::Instant::now() + DRAIN_QUIET;
    loop {
        match cursor.poll() {
            Some(SampleCursorItem::Timed { .. } | SampleCursorItem::Sparse { .. }) => {
                count += 1;
            }
            Some(_) => {}
            None => {
                if tokio::time::Instant::now() >= deadline {
                    return count;
                }
                tokio::task::yield_now().await;
            }
        }
    }
}

/// A live session accepts a valid manifest; a session whose handshake
/// deadline already passed refuses the same object instead of reporting
/// success.
#[tokio::test]
async fn the_running_health_guard_accepts_a_live_session_and_refuses_a_stopped_one() {
    let feed = feed();
    let live = start_with(|route| route, never_times_out()).await;
    assert_eq!(
        post(&live, "/live/manifest.mpd", feed.mpd.as_bytes()).await,
        200,
        "a live session accepts a valid manifest"
    );
    live.task.abort();

    let stopped = start_with(
        |route| route,
        HandshakePolicy::establish_by(Timestamp::ZERO),
    )
    .await;
    assert_eq!(
        status(stopped.addr, http_request("POST", "/live/probe", b"")).await,
        500,
        "a stopped driver must not report an accepted object"
    );
    stopped.task.abort();
}

/// Paths outside the publishing point are 404; the publishing point itself
/// is served (an empty probe here) rather than treated as outside.
#[tokio::test]
async fn paths_are_checked_against_the_publishing_point() {
    let rig = start_with(|route| route, never_times_out()).await;
    assert_eq!(
        status(rig.addr, http_request("POST", "/live-other/x", b"")).await,
        404,
        "a sibling path with the prefix as a plain prefix is outside"
    );
    assert_eq!(
        post(&rig, "/live", b"").await,
        200,
        "the publishing point itself is served"
    );
    rig.task.abort();
}

/// A client that half-closes its write side once the request is sent must
/// still receive the answer: finishing the request is not leaving the
/// connection, and a connection dropped then loses the status code.
#[tokio::test]
async fn a_half_closed_client_still_receives_its_status() {
    let rig = start_with(|route| route, never_times_out()).await;
    for round in 0..HALF_CLOSE_ROUNDS {
        assert_eq!(
            status(rig.addr, http_request("POST", "/live-other/x", b"")).await,
            404,
            "refused request {round} must be answered after the half-close"
        );
        assert_eq!(
            post(&rig, "/live", b"").await,
            200,
            "served request {round} must be answered after the half-close"
        );
    }
    rig.task.abort();
}

/// A body several socket buffers deep is still arriving when the route
/// refuses the path. The refusal must still reach the client: cut the
/// connection with that body unread and the kernel resets it, so a real
/// client never learns why its upload was refused.
#[tokio::test]
async fn a_refused_path_answers_a_body_that_is_still_in_flight() {
    let rig = start_with(|route| route, never_times_out()).await;
    let mut socket = tokio::time::timeout(GUARD, TcpStream::connect(rig.addr))
        .await
        .expect("connect must not hang")
        .expect("connect");
    socket
        .write_all(&declared_request(
            "POST",
            "/live-other/x",
            REFUSED_BODY_BYTES,
        ))
        .await
        .expect("write request head");
    let chunk = vec![0u8; REFUSED_BODY_CHUNK];
    let mut sent = 0;
    let mut write = Ok(());
    while sent < REFUSED_BODY_BYTES {
        write = socket.write_all(&chunk).await;
        if write.is_err() {
            break;
        }
        sent += chunk.len();
        tokio::task::yield_now().await;
    }
    let mut response = Vec::new();
    let read = tokio::time::timeout(GUARD, socket.read_to_end(&mut response))
        .await
        .expect("read must not hang");
    assert!(
        write.is_ok(),
        "the refused body must be read while it arrives: {write:?} after {sent} bytes"
    );
    assert!(
        read.is_ok(),
        "the refusal must be delivered, not reset: {read:?}"
    );
    assert_eq!(
        status_of(&response),
        Some(404),
        "a path outside the publishing point is refused however large its body"
    );
    let _ = socket.shutdown().await;
    rig.task.abort();
}

/// The streamed-segment cap is exact and cumulative across body frames.
#[tokio::test]
async fn the_segment_body_cap_is_exact_and_cumulative() {
    let feed = feed();
    let largest = feed
        .reps
        .iter()
        .map(|rep| rep.init.len().max(rep.segment.len()))
        .max()
        .unwrap_or(0);
    let limit = feed.mpd.len().max(largest) + 64;
    let rig = start_with(
        |route| {
            route
                .with_max_object_bytes(limit)
                .with_max_buffered_object_bytes(limit)
        },
        never_times_out(),
    )
    .await;
    assert_eq!(
        post(&rig, "/live/manifest.mpd", feed.mpd.as_bytes()).await,
        200
    );
    for rep in &feed.reps {
        assert_eq!(
            post(&rig, &format!("/live/init-stream{}.m4s", rep.id), &rep.init).await,
            200,
            "representation {} header",
            rep.id
        );
    }

    let half = limit / 2;
    let first = vec![0u8; half];
    let second = vec![0u8; limit - half];
    let exact = status(
        rig.addr,
        chunked_request("PUT", "/live/chunk-stream1-2.m4s", &[&first, &second]),
    )
    .await;
    assert_ne!(exact, 413, "exactly the cap is not too large");

    let over_first = vec![0u8; half + 1];
    let over_second = vec![0u8; half + 1];
    let over = status(
        rig.addr,
        chunked_request(
            "PUT",
            "/live/chunk-stream1-3.m4s",
            &[&over_first, &over_second],
        ),
    )
    .await;
    assert_eq!(over, 413, "the cap is cumulative across frames");

    let declared_at = status(
        rig.addr,
        declared_request("POST", "/live/manifest.mpd", limit),
    )
    .await;
    assert_ne!(
        declared_at, 413,
        "a declared length exactly at the cap is not too large"
    );
    let declared_over = status(
        rig.addr,
        declared_request("POST", "/live/manifest.mpd", limit + 1),
    )
    .await;
    assert_eq!(
        declared_over, 413,
        "a declared length over the cap is refused before the body"
    );
    rig.task.abort();
}

/// A segment repeated with the same `$Number$` is a duplicate: it is
/// accepted, but publishes no further samples. Deleting the `Segment` arm
/// that carries the number/time would publish them again.
#[tokio::test]
async fn a_repeated_segment_number_publishes_nothing() {
    let feed = feed();
    let largest = feed
        .reps
        .iter()
        .map(|rep| rep.init.len().max(rep.segment.len()))
        .max()
        .unwrap_or(0);
    let limit = feed.mpd.len().max(largest) + 1024;
    let rig = start_with(
        |route| route.with_max_object_bytes(limit),
        never_times_out(),
    )
    .await;
    assert_eq!(
        post(&rig, "/live/manifest.mpd", feed.mpd.as_bytes()).await,
        200
    );
    for rep in &feed.reps {
        assert_eq!(
            post(&rig, &format!("/live/init-stream{}.m4s", rep.id), &rep.init).await,
            200,
            "representation {} header",
            rep.id
        );
    }
    let trunk = tokio::time::timeout(GUARD, rig.handle.await_first_trunk())
        .await
        .expect("the program must be announced")
        .clone();
    let mut cursor = trunk.subscribe();
    let rep = &feed.reps[0];
    assert!(!rep.segment.is_empty());
    assert_eq!(
        post(&rig, "/live/chunk-stream1-1.m4s", &rep.segment).await,
        200,
        "the first segment is accepted"
    );
    assert_eq!(
        drain_samples(&mut cursor).await,
        rep.samples,
        "the first segment publishes its samples"
    );
    assert_eq!(
        post(&rig, "/live/chunk-stream1-1.m4s", &rep.segment).await,
        200,
        "a duplicate segment is accepted"
    );
    assert_eq!(
        drain_samples(&mut cursor).await,
        0,
        "a repeated segment number publishes nothing"
    );
    rig.task.abort();
}
