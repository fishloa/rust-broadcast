//! Test-only mock HTTP auth server (issue #663 "Finish client-side
//! multi-scheme auth"; dedup issue #724): an axum middleware layer that
//! gates a router behind Basic (RFC 7617), Digest (RFC 7616), or Bearer (RFC
//! 6750) auth, for `source::ts_http`/`source::hls_pull`'s own loopback tests
//! to drive the *real* `TsHttpRoute`/`HlsPullRoute` (and therefore the
//! real `source::http_auth`/`hls_runtime::client::tokio_client` challenge-
//! response code) against.
//!
//! A thin wrapper over the real [`broadcast_auth::Verifier`] — the same
//! production challenge/verify code `crate::origin::output_auth_gate` gates
//! every media output route with (see that function). Before issue #724 this
//! module hand-rolled its own independent RFC 7616 §3.4.1 Digest computation
//! (a small, deliberate exception to "no hand-rolled digest in multimux",
//! since `http-auth` — this workspace's Digest client library — has no
//! server-side counterpart); that computation had no request-target check at
//! all (it hashed whatever `uri` the client claimed, unconditionally), so it
//! predated and never exercised `broadcast_auth::Verifier`'s
//! absolute-vs-origin-form `uri`-match fix (issue #724). Wrapping the real
//! `Verifier` here means the mock server now proves the exact same code path
//! multimux's real output-auth middleware runs, including that fix, rather
//! than a second, independently-behaving oracle.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use broadcast_auth::{AuthResult, Credentials, RequestContext, Verifier};

/// Which scheme a [`require_auth`]-wrapped router demands, and the
/// credentials it demands them for.
///
/// Digest carries no `nonce` field (unlike the pre-#724 hand-rolled version):
/// [`Verifier`] issues a fresh, time-limited nonce on every challenge (see
/// its "Nonce handling" module doc) rather than one fixed nonce for the
/// verifier's whole lifetime, which is exactly what a real client answering a
/// real challenge needs — the mock server's nonce value itself is never
/// asserted on by any test.
pub(crate) enum MockAuthScheme {
    /// HTTP Basic (RFC 7617).
    Basic { username: String, password: String },
    /// HTTP Digest (RFC 7616), `qop=auth`/`algorithm=MD5` — the shape
    /// [`crate::source::http_auth`]'s client answers.
    Digest {
        username: String,
        password: String,
        realm: String,
    },
    /// Bearer (RFC 6750) — config-supplied only, never from URL userinfo.
    Bearer { token: String },
}

impl MockAuthScheme {
    /// Builds the one [`Verifier`] this scheme's requests are gated by (see
    /// [`require_auth`] for why this is built once, not per-request).
    fn into_verifier(self) -> Verifier {
        match self {
            MockAuthScheme::Basic { username, password } => {
                Verifier::new(Credentials::Basic { username, password }, "mock")
            }
            MockAuthScheme::Digest {
                username,
                password,
                realm,
            } => Verifier::new(Credentials::Digest { username, password }, realm),
            MockAuthScheme::Bearer { token } => Verifier::new(Credentials::bearer(token), "mock"),
        }
    }
}

/// Wraps `router` behind `scheme`: every request needs a valid
/// `Authorization` matching `scheme`'s credentials, or gets a `401` +
/// `WWW-Authenticate` challenge naming it.
///
/// Builds `scheme`'s [`Verifier`] exactly once (wrapped in an `Arc` so axum's
/// `State` extractor can cheaply clone the handle per request) — a Digest
/// verifier's nonce secret is generated at construction and must stay stable
/// across every challenge and the client's follow-up retry, so this must not
/// rebuild a fresh `Verifier` (and therefore a fresh secret) on every
/// request.
pub(crate) fn require_auth(router: Router, scheme: MockAuthScheme) -> Router {
    let verifier = Arc::new(scheme.into_verifier());
    router.layer(middleware::from_fn_with_state(verifier, auth_gate))
}

async fn auth_gate(State(verifier): State<Arc<Verifier>>, req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let uri = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let headers: Vec<(&str, &str)> = req
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str(), v)))
        .collect();
    let ctx = RequestContext::new(&method, &uri).with_headers(&headers);
    if verifier.verify(&ctx) == AuthResult::Ok {
        return next.run(req).await;
    }
    // `challenge_for` (not `challenge`): matches the real production gate
    // (`crate::origin::output_auth_gate`) this mock stands in for, so an
    // expired nonce is answered `stale=true` here too.
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, verifier.challenge_for(&ctx))],
        Body::empty(),
    )
        .into_response()
}

pub(crate) async fn body_string(resp: Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Counts the timed samples currently readable from `cursor`.
pub(crate) fn drain(cursor: &mut media_plane::trunk::SampleCursor) -> usize {
    let mut n = 0;
    while let Some(item) = cursor.poll() {
        if matches!(item, media_plane::trunk::SampleCursorItem::Timed { .. }) {
            n += 1;
        }
    }
    n
}

pub(crate) fn video_spec(track_id: transmux::TrackId) -> transmux::pipeline::TrackSpec {
    transmux::pipeline::TrackSpec::new(
        track_id,
        90_000,
        transmux::pipeline::CodecConfig::Vp8 {
            width: 1280,
            height: 720,
        },
    )
}

/// Frames per closed segment of [`ingest_run`]'s stream: a sync sample every
/// 30 frames at 30 fps, one second per segment.
pub(crate) const INGEST_GOP_FRAMES: u32 = 30;

/// Pump one source run through the real driver and segmenter into `route`:
/// `segments` GOPs of video, of which the last stays open, as when a live
/// source drops. Returns the driver so the caller can release it.
pub(crate) async fn ingest_run(
    route: &crate::route::RouteHandle,
    segments: u32,
) -> media_plane::ingress::IngestDriver<crate::source::ts_program::TsIngestSession> {
    use crate::source::ts_program::TsIngestSession;
    use media_plane::ingress::{HandshakePolicy, IngestDriver};

    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        crate::source::pull_trunk_config(8),
        HandshakePolicy::establish_by(broadcast_common::Timestamp::from_nanos(u64::MAX)),
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    driver.feed(
        &crate::source::ts_program::test_support::build_ts_bytes_with_gop(
            transmux::TrackId::new(1),
            0xAB,
            segments * INGEST_GOP_FRAMES,
            INGEST_GOP_FRAMES,
        ),
        broadcast_common::Timestamp::ZERO,
    );
    let mut progress = crate::source::DriverProgress::new();
    crate::source::advance_route(&driver, route, &mut progress).await;
    driver
}

pub(crate) fn app_with(
    route: &std::sync::Arc<crate::route::RouteHandle>,
    output: std::sync::Arc<dyn crate::output::Output>,
) -> axum::Router {
    let mut streams = std::collections::HashMap::new();
    streams.insert("cam".to_string(), (route.clone(), vec![output]));
    crate::origin::router(std::sync::Arc::new(crate::origin::AppState::new(streams)))
}

pub(crate) async fn get_bytes(app: &axum::Router, path: &str) -> (axum::http::StatusCode, Vec<u8>) {
    use tower::ServiceExt;
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec();
    (status, bytes)
}

/// The decode time of the first sample of the served segment `seq`: its
/// `tfdt`, read by demuxing the init and the segment together.
pub(crate) async fn served_first_dts(app: &axum::Router, seq: u64) -> i64 {
    use broadcast_common::Unpackage;
    let (status, init) = get_bytes(app, "/cam/init-1.mp4").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let (status, segment) = get_bytes(app, &format!("/cam/seg-1-{seq}.m4s")).await;
    assert_eq!(status, axum::http::StatusCode::OK, "segment {seq}");
    let media = transmux::Fmp4Demux::new()
        .unpackage(&[init, segment].concat())
        .expect("demux");
    media.tracks[0].samples[0].dts.expect("dts")
}

pub(crate) fn mpd_attribute(xml: &str, key: &str) -> String {
    let at = xml.find(key).unwrap_or_else(|| panic!("{key} in {xml}")) + key.len();
    xml[at..].split('"').next().expect("quote").to_string()
}

/// A source reconnect on a route serving `output` at `manifest_path`: the
/// numbers keep counting, `availabilityStartTime`, `startNumber` and the
/// Period are unchanged, every advertised segment is served and its `tfdt` is
/// the time its number implies (`tfdt(1)` plus whole segments; the stream has
/// a one-second GOP and the lost open segment's slot stays empty).
pub(crate) async fn assert_dash_continues_across_a_reconnect(
    output: std::sync::Arc<dyn crate::output::Output>,
    manifest_path: &str,
) {
    use axum::http::StatusCode;
    const SEGMENT_TICKS: i64 = 90_000;
    let route = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 500, 8));
    let app = app_with(&route, output);

    let first = ingest_run(&route, 8).await;
    let (status, before) = get_bytes(&app, manifest_path).await;
    assert_eq!(status, StatusCode::OK);
    let before = String::from_utf8(before).expect("utf8");
    let t0 = served_first_dts(&app, 1).await;
    let old_end = served_first_dts(&app, 7).await + SEGMENT_TICKS;

    crate::source::release_route(&first, &route);
    let _second = ingest_run(&route, 8).await;
    let (status, after) = get_bytes(&app, manifest_path).await;
    assert_eq!(status, StatusCode::OK);
    let after = String::from_utf8(after).expect("utf8");

    for key in ["availabilityStartTime=\"", "startNumber=\""] {
        assert_eq!(mpd_attribute(&before, key), mpd_attribute(&after, key));
    }
    let period = |xml: &str| {
        let at = xml.find("<Period").expect("Period");
        xml[at..].split('>').next().expect("tag").to_string()
    };
    assert_eq!(period(&before), period(&after));

    let start: i64 = mpd_attribute(&after, "startNumber=\"")
        .parse()
        .expect("number");
    let ast: jiff::Timestamp = mpd_attribute(&after, "availabilityStartTime=\"")
        .parse()
        .expect("AST");
    let published: jiff::Timestamp = mpd_attribute(&after, "publishTime=\"")
        .parse()
        .expect("publishTime");
    let newest = start + (published.as_second() - ast.as_second()) - 1;
    let mut previous = old_end;
    for n in 9..=newest {
        let dts = served_first_dts(&app, u64::try_from(n).expect("positive")).await;
        let advertised = t0 + (n - start) * SEGMENT_TICKS;
        assert!(
            (dts - advertised).abs() < SEGMENT_TICKS,
            "segment {n}: tfdt {dts} is not the advertised time {advertised}"
        );
        assert!(
            dts >= previous,
            "segment {n}: tfdt {dts} moved back from {previous}"
        );
        previous = dts + SEGMENT_TICKS;
    }
    assert!(
        (14..=15).contains(&newest),
        "the MPD advertises the new run's last segment, not {newest}"
    );
}
