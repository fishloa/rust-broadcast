//! MPEG-2 Transport Stream over HTTP ingest source: a streaming HTTP GET driven by a
//! [`broadcast_client::Session`] in relay mode through the shared
//! `crate::source::client_host`.
//!
//! The session owns the HTTP protocol (redirects, Basic/Digest/Bearer auth, the finite/VOD
//! decision and the post-establishment stall policy of `broadcast_client::tshttp`), the host only
//! performs its requests, and the programme split of a multi-programme stream is
//! `EventIngest`. A session that asks to reconnect ends the attempt;
//! the route supervisor owns the backoff.

use std::time::Duration;

use broadcast_client::tshttp::Config as PolicyConfig;
use broadcast_client::{Config, SourceKind};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;

use crate::error::Result;
use crate::source::IngestTimeouts;
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep};
use crate::source::pull_target::{PullKind, PullRoute};

/// An MPEG-2 TS-over-HTTP route: an `http(s)://` URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`). Replaces the old (pre-5a)
/// `TsHttpSource`; [`run_ts_http`] is the new drive loop.
pub type TsHttpRoute = PullRoute<TsHttpKind>;

/// The [`PullKind`] of a [`TsHttpRoute`].
#[derive(Debug, Clone, Copy)]
pub struct TsHttpKind;

impl PullKind for TsHttpKind {
    const ROUTE: &'static str = "TsHttpRoute";
    type Extra = ();
}

impl crate::source::pull_target::PullDrive for TsHttpKind {
    const SEGMENTED: bool = false;

    async fn drive(
        route: std::sync::Arc<crate::source::pull_target::PullRoute<Self>>,
        trunk_config: media_plane::trunk::TrunkConfig,
        handshake: media_plane::ingress::HandshakePolicy,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        run_ts_http(&route, trunk_config, handshake, &route_handle, cancel).await
    }
}

const MIN_START_TIMEOUT_MS: u64 = 1;
const STALL_NOTICE_DIVISOR: u64 = 2;
const MIN_STALL_RECONNECT_MS: u64 = 2;
const WHAT: &str = "ts/http";

fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The policy-core configuration for a route: the post-establishment stall
/// threshold is the route's read timeout, the core's start timeout is the
/// route's connect budget (the driver is `Live` from the first byte, so the
/// handshake deadline alone never bounds a connection that sends bytes but no
/// media), and backoff is unused because the supervisor owns reconnect.
pub(crate) fn policy_config(timeouts: IngestTimeouts) -> PolicyConfig {
    let reconnect = duration_ms(timeouts.read).max(MIN_STALL_RECONNECT_MS);
    PolicyConfig {
        start_timeout_ms: duration_ms(timeouts.connect).max(MIN_START_TIMEOUT_MS),
        stall_notice_ms: reconnect / STALL_NOTICE_DIVISOR,
        stall_reconnect_ms: reconnect,
        ..PolicyConfig::default()
    }
}

/// Opens `route`'s GET in a relay-mode session and drives it until the body ends or a read
/// fails.
///
/// Returns `Ok(())` on a clean end-of-body (the driver is left
/// [`media_plane::ingress::HealthState::Ended`]) and `Err` on a genuine
/// failure — the distinction the route supervisor should act on differently.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::report_driver_progress` for what it does each iteration.
pub async fn run_ts_http(
    route: &TsHttpRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let timeouts = route.target.timeouts;
    let mut config = Config::relay();
    config.kind = SourceKind::TsHttp;
    config.max_queued_bytes = crate::source::LIVE_EVENT_QUEUE_BYTES;
    config.ts_http = policy_config(timeouts);
    let mut step = SessionStep::open(
        &route.target.url,
        config,
        route.target.auth.clone(),
        SessionLabels::new(WHAT, timeouts),
        std::sync::Arc::clone(route_handle),
        true,
    )?;
    let driver = IngestDriver::new(
        EventIngest::new(),
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut progress = crate::source::DriverProgress::new();
    let mut sink = tokio::io::sink();
    let err = crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        &mut progress,
        cancel,
        timeouts,
        crate::source::driver::DEFAULT_WRITE_TIMEOUT,
        &mut sink,
        &mut step,
        |never: std::convert::Infallible| match never {},
    )
    .await;
    match err {
        crate::error::MultimuxError::Connect { reason } if reason.starts_with("session ended") => {
            Ok(())
        }
        other => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MultimuxError;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use axum::Router;
    use axum::body::Body;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use broadcast_auth::Credentials;
    use media_plane::trunk::SampleCursorItem;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use transmux::TrackId;

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "ts-http-bearer-token";
    const RUN_CEILING: Duration = Duration::from_secs(60);
    const TRUNK_WAIT: Duration = Duration::from_secs(5);
    const HANDSHAKE_BUDGET: Duration = Duration::from_secs(1);
    const HANDSHAKE_MARGIN: Duration = Duration::from_secs(4);
    const LONG_READ: Duration = Duration::from_secs(30);
    const TRICKLE_PERIOD: Duration = Duration::from_millis(50);

    async fn start_chunked_ts_server_with_auth(
        body: Vec<u8>,
        auth: Option<crate::testutil::MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(body: axum::extract::State<Vec<u8>>) -> AxumResponse {
            let chunks: Vec<std::result::Result<Vec<u8>, std::io::Error>> =
                body.0.chunks(7 * 188).map(|c| Ok(c.to_vec())).collect();
            let body = Body::from_stream(futures_util::stream::iter(chunks));
            ([(axum::http::header::CONTENT_TYPE, "video/mp2t")], body).into_response()
        }
        let mut app = Router::new()
            .route("/stream.ts", get(handler))
            .with_state(body);
        if let Some(scheme) = auth {
            app = crate::testutil::require_auth(app, scheme);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/stream.ts"), server)
    }

    async fn start_chunked_ts_server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        start_chunked_ts_server_with_auth(body, None).await
    }

    async fn run(
        route: &TsHttpRoute,
        handshake: HandshakePolicy,
        ceiling: Duration,
    ) -> (Result<()>, std::sync::Arc<crate::route::RouteHandle>) {
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let result = tokio::time::timeout(
            ceiling,
            run_ts_http(
                route,
                trunk_config(),
                handshake,
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_ts_http must finish inside the ceiling");
        (result, route_handle)
    }

    async fn sample_count(route_handle: &crate::route::RouteHandle) -> usize {
        let Ok(trunk) = tokio::time::timeout(TRUNK_WAIT, route_handle.await_first_trunk()).await
        else {
            return 0;
        };
        let mut cursor = trunk.subscribe_from_backlog();
        let mut count = 0;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                count += 1;
            }
        }
        count
    }

    async fn pull(route: TsHttpRoute) -> Result<usize> {
        let (result, route_handle) = run(&route, handshake(), RUN_CEILING).await;
        result?;
        Ok(sample_count(&route_handle).await)
    }

    #[tokio::test]
    async fn loopback_http_ts_yields_samples_and_ends_cleanly() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server(ts_bytes).await;
        let route = TsHttpRoute::new("cam-ts-http", url);
        let (result, route_handle) = run(&route, handshake(), RUN_CEILING).await;
        assert!(
            result.is_ok(),
            "the chunked server's body must end cleanly: {result:?}"
        );
        assert!(
            sample_count(&route_handle).await > 0,
            "expected at least one sample from the muxed TS stream over HTTP in the Trunk"
        );
        server.abort();
    }

    #[tokio::test]
    async fn connect_fails_on_non_success_status() {
        let app = Router::new().route(
            "/nope.ts",
            get(|| async { axum::http::StatusCode::NOT_FOUND }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        let route = TsHttpRoute::new("cam-ts-http", format!("http://{addr}/nope.ts"));
        let (result, _) = run(&route, handshake(), RUN_CEILING).await;
        assert!(
            matches!(result, Err(MultimuxError::Connect { .. })),
            "a 404 must fail the route: {result:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_mid_body() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let _server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\n\r\n",
                ts_bytes.len() * 10
            );
            stream
                .write_all(header.as_bytes())
                .await
                .expect("write header");
            stream
                .write_all(&ts_bytes)
                .await
                .expect("write body prefix");
            std::future::pending::<()>().await;
        });
        let route = TsHttpRoute::new("stalled", format!("http://{addr}/stream.ts")).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_secs(2),
            },
        );
        let (result, _) = run(&route, handshake(), Duration::from_secs(10)).await;
        assert!(
            matches!(result, Err(MultimuxError::Connect { .. })),
            "a server that goes silent mid-body must fail, not hang forever: {result:?}"
        );
    }

    async fn pre_pmt_server(trickle: bool) -> std::net::SocketAddr {
        let pat_only =
            build_ts_bytes(TrackId::new(1), 0xAB, 10)[..mpeg_ts::ts::TS_PACKET_SIZE].to_vec();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .expect("write header");
            let chunk = [
                format!("{:x}\r\n", pat_only.len()).as_bytes(),
                &pat_only,
                b"\r\n",
            ]
            .concat();
            loop {
                if stream.write_all(&chunk).await.is_err() || !trickle {
                    break;
                }
                tokio::time::sleep(TRICKLE_PERIOD).await;
            }
            std::future::pending::<()>().await;
        });
        addr
    }

    async fn run_against(addr: std::net::SocketAddr) -> (Result<()>, Duration) {
        let route = TsHttpRoute::new("pre-pmt", format!("http://{addr}/stream.ts")).with_timeouts(
            IngestTimeouts {
                connect: HANDSHAKE_BUDGET,
                read: LONG_READ,
            },
        );
        let started = std::time::Instant::now();
        let (result, _) = run(
            &route,
            crate::source::handshake_policy(HANDSHAKE_BUDGET),
            HANDSHAKE_BUDGET + HANDSHAKE_MARGIN,
        )
        .await;
        (result, started.elapsed())
    }

    fn assert_handshake_timeout(result: &Result<()>) {
        match result {
            Err(MultimuxError::Connect { reason }) => {
                assert!(reason.contains("handshake timed out"), "{reason}");
            }
            other => panic!("expected a handshake timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_server_that_goes_silent_before_the_pmt_fails_at_the_handshake_deadline() {
        let addr = pre_pmt_server(false).await;
        let (result, elapsed) = run_against(addr).await;
        assert_handshake_timeout(&result);
        assert!(elapsed < LONG_READ, "{elapsed:?}");
    }

    #[tokio::test]
    async fn a_server_that_keeps_sending_without_ever_sending_a_pmt_fails_at_the_handshake_deadline()
     {
        let addr = pre_pmt_server(true).await;
        let (result, elapsed) = run_against(addr).await;
        assert_handshake_timeout(&result);
        assert!(elapsed < LONG_READ, "{elapsed:?}");
    }

    #[tokio::test]
    async fn live_eof_without_length_stays_a_clean_end() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 4);
        let (url, server) = start_chunked_ts_server(ts_bytes).await;
        let total = pull(TsHttpRoute::new("live-eof", url))
            .await
            .expect("a live body that ends without a length is a clean end");
        assert!(total > 0);
        server.abort();
    }

    #[tokio::test]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);
        let total = pull(TsHttpRoute::new("cam-basic", credentialed))
            .await
            .expect("Basic auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Basic auth");
        server.abort();
    }

    #[tokio::test]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);
        let total = pull(TsHttpRoute::new("cam-digest", credentialed))
            .await
            .expect("Digest auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Digest auth");
        server.abort();
    }

    #[tokio::test]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Bearer {
                token: BEARER_TOKEN.into(),
            }),
        )
        .await;
        let route =
            TsHttpRoute::new("cam-bearer", url).with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        let total = pull(route)
            .await
            .expect("config-supplied Bearer token must authenticate");
        assert!(total > 0, "expected real samples after Bearer auth");
        server.abort();
    }

    #[tokio::test]
    async fn config_auth_overrides_wrong_url_userinfo() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_url_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);
        let route = TsHttpRoute::new("cam-override", wrong_url_creds)
            .with_auth(Some(Credentials::new(AUTH_USER, AUTH_PASS)));
        let total = pull(route)
            .await
            .expect("config auth must override the URL's wrong userinfo password");
        assert!(
            total > 0,
            "expected real samples via config-overridden auth"
        );
        server.abort();
    }

    #[tokio::test]
    async fn wrong_credentials_stay_401_and_connect_errors() {
        let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Digest {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
                realm: DIGEST_REALM.into(),
            }),
        )
        .await;
        let wrong_creds = url.replacen("http://", &format!("http://{AUTH_USER}:wrongpass@"), 1);
        let (result, _) = run(
            &TsHttpRoute::new("cam-wrong", wrong_creds),
            handshake(),
            RUN_CEILING,
        )
        .await;
        assert!(
            result.is_err(),
            "wrong credentials must fail the route, not silently proceed: {result:?}"
        );
        server.abort();
    }
}
