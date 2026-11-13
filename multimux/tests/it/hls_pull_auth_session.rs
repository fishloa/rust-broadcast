//! Cookie and Digest-challenge acceptance through the production HLS pull
//! (issue #2468): a real `run_hls_pull` against a loopback origin that keeps
//! server-side state, proving the pull session keeps an origin's `Set-Cookie`
//! for its segment requests, and pulls a segment that is only served to the
//! credentials its playlist exchange negotiated.
//!
//! Every origin here serves the committed `ts-hls` fixture's classic TS
//! segments behind a VOD media playlist whose last segment closes it, so a
//! pull plays to its own end and reports its own result. Each test runs on its
//! own origin and its own route, so two pulls never share a media timeline.
//!
//! Both tests bite on whether the pull plays at all. A pull that builds a
//! fresh auth state per request never learns the playlist's cookie, so the
//! origin refuses every segment; a pull whose segment fetch cannot present the
//! credentials the playlist negotiated is refused the same way. Either way the
//! route publishes no media.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier};
use broadcast_common::Timestamp;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::route::RouteHandle;
use multimux::source::hls_pull::{HlsPullRoute, run_hls_pull};

use crate::support::listener::bind_tcp;

/// The session cookie the cookie origin hands out and then requires.
const SESSION_COOKIE: &str = "pullsid=3f9a-session";
/// How many `ts-hls` fixture segments each origin serves.
const SEGMENTS: u8 = 2;
/// The leading PSI packets of a `ts-hls` fixture segment that every consumer
/// of the fixture drops (`hls_pull_keys.rs` does the same).
const TS_PACKET_LEN: usize = 188;
const LEADING_PACKETS_DROPPED: usize = 3;
/// The media sequence every playlist starts at: each pull has its own origin
/// and its own route, so no two pulls share a sequence.
const SEQUENCE: u64 = 10_000;
/// Digest credentials and realm the challenged origin judges.
const DIGEST_REALM: &str = "challenged origin";
const DIGEST_USER: &str = "cam-user";
const DIGEST_PASSWORD: &str = "cam-pass";
/// How long the pull may take before the test calls it stalled. A VOD pull
/// ends by itself once its playlist is complete; this only catches a deadlock.
const RUN_CEILING: Duration = Duration::from_secs(60);
/// How long one look at the route's trunk waits before the loop moves on.
/// How long the route may take to publish its first media once it has a trunk.
const WAIT_FOR_MEDIA: Duration = Duration::from_secs(30);
/// Trunk sizing for the pulled route: a few seconds of a real TS fixture.
const TIMED_RING: usize = 4096;
const RING: usize = 16;
/// The route's own segmenter: one-second segments, so the route closes the
/// fixture's samples into segments of its own.
const ROUTE_TARGET_DURATION_SECS: f64 = 1.0;
const ROUTE_PART_TARGET_MS: u32 = 200;
const ROUTE_WINDOW_SEGMENTS: usize = 4;

fn nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("test capacity must be non-zero")
}

/// One segment of the `ts-hls` fixture, its leading PSI packets dropped.
fn seg(index: u8) -> Vec<u8> {
    let name = format!("hls/ts-hls/index{index}.ts");
    test_fixtures::read(&name)[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..].to_vec()
}

/// A VOD Media Playlist naming `seg0.ts`..`seg{count-1}.ts`.
fn playlist(count: u8) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{SEQUENCE}\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n"
    );
    for index in 0..count {
        text.push_str(&format!("#EXTINF:2.000000,\nseg{index}.ts\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

/// What one origin saw.
#[derive(Default)]
struct Log {
    /// Segment requests that carried the session cookie.
    with_cookie: AtomicU64,
    /// Requests answered with a Digest challenge.
    challenges: AtomicU64,
}

/// One origin as the test sees it after standup: its base URL and what it
/// recorded.
struct Origin {
    base: String,
    log: Arc<Log>,
}

/// What the router sees: one handle per field the handler writes.
#[derive(Clone)]
struct Shared {
    log: Arc<Log>,
    verifier: Arc<Verifier>,
}

impl Shared {
    /// The fixture segment `seg{index}.ts` names, if it names one.
    fn segment_index(segment: &str) -> Option<u8> {
        let index: u8 = segment
            .strip_prefix("seg")?
            .strip_suffix(".ts")?
            .parse()
            .ok()?;
        (index < SEGMENTS).then_some(index)
    }

    /// Serve `body` to a request whose Digest answer verifies, and challenge
    /// anything else.
    fn digest_or_challenge(&self, headers: &HeaderMap, target: &str, body: Vec<u8>) -> Response {
        let header_list: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().unwrap_or_default()))
            .collect();
        let ctx = RequestContext::new("GET", target).with_headers(&header_list);
        if let AuthResult::Ok = self.verifier.verify(&ctx) {
            return body.into_response();
        }
        self.log.challenges.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, self.verifier.challenge_for(&ctx))],
        )
            .into_response()
    }
}

async fn handler(State(origin): State<Shared>, uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path();
    let (origin_name, resource) = path
        .trim_start_matches('/')
        .split_once('/')
        .unwrap_or(("", path));
    match (origin_name, resource) {
        ("cookie", "media.m3u8") => (
            StatusCode::OK,
            [(header::SET_COOKIE, format!("{SESSION_COOKIE}; Path=/"))],
            playlist(SEGMENTS),
        )
            .into_response(),
        ("cookie", segment) => {
            let carries = headers
                .get(header::COOKIE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split("; ").any(|pair| pair == SESSION_COOKIE));
            match Shared::segment_index(segment) {
                Some(index) if carries => {
                    origin.log.with_cookie.fetch_add(1, Ordering::SeqCst);
                    seg(index).into_response()
                }
                Some(_) => StatusCode::FORBIDDEN.into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }
        ("challenged", "media.m3u8") => {
            origin.digest_or_challenge(&headers, path, playlist(SEGMENTS).into_bytes())
        }
        // Every segment is judged: the playlist's own exchange has to have
        // landed for even the first one to be served.
        ("challenged", segment) => match Shared::segment_index(segment) {
            Some(index) => origin.digest_or_challenge(&headers, path, seg(index)),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Starts the loopback origin.
async fn serve() -> Origin {
    let log = Arc::new(Log::default());
    let (addr, tcp) = bind_tcp();
    let verifier = Arc::new(Verifier::new(
        Credentials::Digest {
            username: DIGEST_USER.to_string(),
            password: DIGEST_PASSWORD.to_string(),
        },
        DIGEST_REALM,
    ));
    let app = Router::new().fallback(handler).with_state(Shared {
        log: Arc::clone(&log),
        verifier,
    });
    tokio::spawn(async move {
        axum::serve(tcp, app).await.expect("axum server");
    });
    Origin {
        base: format!("http://{addr}"),
        log,
    }
}

/// Runs one pull on its own route and reports how many samples the route
/// published, why the pull failed if it did, and how the pull's first media
/// arrived.
///
/// A complete VOD playlist ends the pull by itself, so this waits for the
/// route's first media (the trunk is captured while it is still registered)
/// and then for the pull's own end. A pull that fails first reports that.
async fn played_samples(
    origin: &str,
    path: &str,
    auth: Option<Credentials>,
    name: &str,
) -> Option<String> {
    let route = HlsPullRoute::new(name, format!("{origin}{path}")).with_auth(auth);
    let route_handle = Arc::new(RouteHandle::new(
        ROUTE_TARGET_DURATION_SECS,
        ROUTE_PART_TARGET_MS,
        ROUTE_WINDOW_SEGMENTS,
    ));
    let config = TrunkConfig::default()
        .with_timed_capacity(nz(TIMED_RING))
        .with_sparse_capacity(nz(RING))
        .with_segment_capacity(nz(RING))
        .with_event_capacity(nz(RING))
        .with_part_capacity(nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let pulled = run_hls_pull(
        &route,
        config,
        handshake,
        &route_handle,
        tokio_util::sync::CancellationToken::new(),
    );
    tokio::pin!(pulled);
    // The wait for the first program and for the media behind it is the pull's
    // own; `RUN_CEILING` is a hang guard on the whole call, not a pacing
    // sleep.
    let outcome = tokio::time::timeout(RUN_CEILING, async {
        // The pull has been created but not polled yet, so its first program
        // does not exist until it runs: poll it alongside this wait rather
        // than reading the registry before it has done anything.
        let trunk = tokio::select! {
            result = &mut pulled => return result.err().map(|error| error.to_string()),
            trunk = route_handle.await_first_trunk() => trunk,
        };
        // A route serves what its pull publishes the moment it is published,
        // so a repeated read of the same trunk is a real check of "has anything
        // been published yet", not a fixed wait. The bound only catches a pull
        // that never publishes.
        let sample_outcome = tokio::time::timeout(WAIT_FOR_MEDIA, async {
            loop {
                let mut cursor = trunk.subscribe_from_backlog();
                let mut played = None;
                while let Some(item) = cursor.poll() {
                    if let SampleCursorItem::Timed { sample, .. } = item {
                        played = Some(sample);
                    }
                }
                if let Some(sample) = played {
                    return Ok(sample);
                }
                tokio::select! {
                    biased;
                    result = &mut pulled => return Err(result),
                    () = tokio::task::yield_now() => {}
                }
            }
        })
        .await;
        match sample_outcome {
            Err(_) => return Some("the route published no media".to_string()),
            Ok(Err(result)) => {
                return Some(match result {
                    Ok(()) => "the pull ended before publishing media".to_string(),
                    Err(error) => error.to_string(),
                });
            }
            Ok(Ok(_)) => {}
        }
        match &mut pulled.await {
            Ok(()) => None,
            Err(error) => Some(error.to_string()),
        }
    })
    .await;
    match outcome {
        Ok(failure) => failure,
        Err(_) => Some("the pull stalled".to_string()),
    }
}

/// The cookie an origin sets on the playlist is sent with every segment, and
/// the route plays.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_playlist_cookie_is_sent_with_the_segments_and_the_route_plays() {
    let origin = serve().await;
    let failure = played_samples(&origin.base, "/cookie/media.m3u8", None, "cookie").await;

    assert!(
        failure.is_none(),
        "a route whose origin requires the playlist's cookie must play: {failure:?}"
    );
    assert!(
        origin.log.with_cookie.load(Ordering::SeqCst) > 0,
        "the origin must really have served a segment to a cookie-carrying request"
    );
}

/// A segment served only to the credentials its playlist exchange negotiated is
/// pulled, and the route plays. Biting test: with a fresh auth state per
/// request, the segment is challenged and the route publishes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_segment_behind_a_digest_challenge_is_pulled_after_the_playlist_answers_it() {
    let origin = serve().await;
    let credentials = Credentials::Digest {
        username: DIGEST_USER.to_string(),
        password: DIGEST_PASSWORD.to_string(),
    };
    let failure = played_samples(
        &origin.base,
        "/challenged/media.m3u8",
        Some(credentials),
        "challenged",
    )
    .await;

    assert!(
        failure.is_none(),
        "a segment behind a Digest challenge must be pulled: {failure:?}, {} challenges \
         answered",
        origin.log.challenges.load(Ordering::SeqCst)
    );
    assert!(
        origin.log.challenges.load(Ordering::SeqCst) > 0,
        "the origin must really have challenged a request"
    );
}
