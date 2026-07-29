//! MPEG-2 Transport Stream over HTTP ingest source (issue #663 P3c; ported
//! onto the media-plane ingress traits at plan step 5a): a streaming HTTP
//! GET (chunked/progressive, `reqwest`) feeding the shared
//! [`crate::source::ts_program::TsIngestSession`].
//!
//! Like [`crate::source::ts_udp`], this module owns **only the transport**
//! — all PAT/PMT/PES demuxing and `DemuxEvent`→`SessionEvent` translation
//! (including the B5 mid-stream `NewProgram` handling) lives in
//! [`crate::source::ts_program`], shared verbatim between the three TS
//! transports. Before this port, this file carried its own near-identical
//! copy of that drain loop.
//!
//! # Where the I/O boundary falls, and why `dial()` is still no-I/O
//!
//! Opening the HTTP body stream **is** real I/O — one GET (two if a Digest
//! `401` challenge has to be answered; see
//! `crate::source::http_auth::authenticated_get`). But it is
//! *transport-opening*, not a media handshake: it is the same category as
//! [`crate::source::ts_udp`]'s `UdpSocket::bind` and
//! [`crate::source::rtsp`]'s `TcpStream::connect`, and in this design that
//! has always been the driver's job, never the session's. So
//! [`TsHttpDialer::dial`] constructs the sans-IO session and performs no
//! I/O, and [`open_stream`]/[`run_ts_http`] — the tokio-side driver — does
//! the GET and then feeds body chunks in.
//!
//! **`reqwest` has no sans-IO core** (its `Response::bytes_stream()` is
//! bound to a live hyper connection; there is no bytes-in/bytes-out type),
//! so unlike [`crate::source::rtsp`] — which could delegate its whole
//! multi-round-trip handshake to `rtsp-runtime`'s sans-IO `ClientSession` —
//! there is nothing here to delegate the GET to. That is fine precisely
//! because the GET is not a handshake: nothing about it needs to be
//! expressed through `poll_transmit`/`feed`. Once the body is open, the
//! *entire* remaining protocol is "bytes arrive, demux them", which is
//! exactly `Stage::feed`.
//!
//! Auth itself **is** sans-IO (`broadcast_auth::Authenticator` is pure
//! challenge-string in, `Authorization`-header out) and is unchanged by this
//! port. Credentials come from [`TsHttpRoute::with_auth`] (config-supplied,
//! e.g. a Bearer token — the only way to supply one, since it has no
//! URL-userinfo form) if set, else the connect URL's own userinfo — see
//! `crate::source::http_auth::resolve_credentials`.
//!
//! # End-of-stream is a real, distinct outcome here
//!
//! Unlike UDP (connectionless, never signals end-of-stream), an HTTP
//! response body *does* end. [`recv_and_feed`] reports that as
//! [`StreamStatus::Ended`], which [`run_ts_http`] turns into
//! `IngestDriver::finish()` → [`media_plane::ingress::HealthState::Ended`]
//! — genuinely distinct from [`media_plane::ingress::HealthState::Failed`]
//! for a read stall or a transport error. That distinction is exactly the
//! `HealthState` fix plan step 3c made producible, and this is the first
//! ported source that can actually produce both.

use std::convert::Infallible;
use std::time::Duration;

use broadcast_common::Timestamp;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use media_plane::ingress::{Dialer, HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;
use reqwest::Client;
use url::Url;

use broadcast_auth::Credentials;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use crate::source::ts_program::TsIngestSession;
use crate::source::{IngestTimeouts, Source};

/// An MPEG-2 TS-over-HTTP route: an `http(s)://` URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`). Replaces the old (pre-5a)
/// `TsHttpSource`; [`run_ts_http`] is the new drive loop.
#[derive(Clone)]
pub struct TsHttpRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for TsHttpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TsHttpRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl TsHttpRoute {
    /// Build a route descriptor.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        TsHttpRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }
}

impl Source for TsHttpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Constructs a [`TsIngestSession`] — performs **no I/O** (see the module
/// doc). The GET lives in [`open_stream`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TsHttpDialer;

impl Dialer for TsHttpDialer {
    type Session = TsIngestSession;
    /// Construction cannot fail — URL parsing and the GET both belong to
    /// [`open_stream`], the I/O side.
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<TsIngestSession, Infallible> {
        Ok(TsIngestSession::new())
    }
}

/// The open HTTP response body a [`TsIngestSession`] is fed from.
pub type TsHttpStream = BoxStream<'static, reqwest::Result<Vec<u8>>>;

/// Opens the streaming GET for `route` (answering a `401` Digest challenge
/// if credentials were supplied), returning the response body stream —
/// **transport-opening I/O, deliberately outside `dial()`**; see the module
/// doc.
///
/// # Errors
/// A URL that will not parse, a transport failure, or a non-2xx status
/// (`401` maps to [`MultimuxError::Auth`], anything else to
/// [`MultimuxError::Connect`]).
pub async fn open_stream(route: &TsHttpRoute) -> Result<TsHttpStream> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad TS-over-HTTP URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;

    let client = Client::builder()
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    let response = authenticated_get(&client, clean_url.as_str(), credentials.as_ref()).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
            MultimuxError::Auth {
                reason: format!("ts/http: {status}"),
            }
        } else {
            MultimuxError::Connect {
                reason: format!("ts/http: HTTP {status}"),
            }
        });
    }
    Ok(response
        .bytes_stream()
        .map(|item| item.map(|b| b.to_vec()))
        .boxed())
}

/// What one [`recv_and_feed`] call observed on the body stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamStatus {
    /// A chunk was read and fed to the driver.
    Fed,
    /// The response body ended cleanly — not an error. See the module doc's
    /// "End-of-stream is a real, distinct outcome here".
    Ended,
}

/// Reads the next body chunk (bounded by `read_timeout`) and feeds it to
/// `driver`.
///
/// # Errors
/// A read stall or a transport-level chunk error — both genuine failures,
/// distinct from the clean [`StreamStatus::Ended`].
pub async fn recv_and_feed(
    stream: &mut TsHttpStream,
    driver: &mut IngestDriver<TsIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<StreamStatus> {
    let Some(chunk) = tokio::time::timeout(read_timeout, stream.next())
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("ts/http stream read: no data within {read_timeout:?}"),
        })?
    else {
        return Ok(StreamStatus::Ended);
    };
    let chunk = chunk.map_err(|e| MultimuxError::Connect {
        reason: format!("ts/http stream read: {e}"),
    })?;
    driver.feed(&chunk, now);
    Ok(StreamStatus::Fed)
}

/// Opens `route`'s GET and drives a fresh [`TsIngestSession`] through
/// [`IngestDriver`] until the body ends or a read fails — the new drive
/// loop, replacing the pre-5a `TsHttpSource`/`TsHttpSession` pair.
///
/// Returns `Ok(())` on a clean end-of-body (the driver is left
/// [`media_plane::ingress::HealthState::Ended`]) and `Err` on a genuine
/// failure — the distinction the route supervisor should act on differently.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration.
pub async fn run_ts_http(
    route: &TsHttpRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let mut stream = open_stream(route).await?;
    let mut dialer = TsHttpDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let mut driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let read_timeout = route.timeouts.read;
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();
    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        let status = recv_and_feed(&mut stream, &mut driver, read_timeout, now).await?;
        crate::source::advance_route(&driver, route_handle, &mut progress);
        match status {
            StreamStatus::Fed => {}
            StreamStatus::Ended => {
                driver.finish();
                // Flush every program's trailing buffered partial segment
                // now that the driver is terminal -- `advance_route` above
                // ran while the driver was still `Live`; this call's own
                // internal terminal-health check does the flush.
                crate::source::advance_route(&driver, route_handle, &mut progress);
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use axum::Router;
    use axum::body::Body;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use media_plane::ingress::{HealthState, ProgramId};

    /// Starts a tiny axum server streaming `body` in fixed-size chunks (a
    /// real chunked-transfer HTTP response, not a single buffered body) at
    /// `/stream.ts`, returning its base URL. `auth`, if given, gates every
    /// request behind that scheme (see `crate::testutil::require_auth`) —
    /// used by the auth-scheme biting tests below; `None` (the plain
    /// `start_chunked_ts_server` case) mirrors the original no-auth server.
    async fn start_chunked_ts_server_with_auth(
        body: Vec<u8>,
        auth: Option<crate::testutil::MockAuthScheme>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(body: axum::extract::State<Vec<u8>>) -> AxumResponse {
            // Stream in small chunks so the client genuinely reads the body
            // incrementally (biting on `bytes_stream()`, not just a single
            // buffered read).
            let chunks: Vec<std::result::Result<Vec<u8>, std::io::Error>> =
                body.0.chunks(7 * 188).map(|c| Ok(c.to_vec())).collect();
            let stream = futures_util::stream::iter(chunks);
            let body = Body::from_stream(stream);
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

    /// Starts a plain (no-auth) chunked TS server — see
    /// [`start_chunked_ts_server_with_auth`].
    async fn start_chunked_ts_server(body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        start_chunked_ts_server_with_auth(body, None).await
    }

    /// Loopback biting test: a real axum server streams a real muxed TS
    /// fixture over chunked HTTP; asserts the ported route resolves the track
    /// set and yields real depayloaded samples.
    #[tokio::test]
    async fn loopback_http_ts_yields_samples_after_pmt_resolves() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
        let (url, server) = start_chunked_ts_server(ts_bytes).await;

        let route = TsHttpRoute::new("cam-ts-http", url);
        // HANG GUARD (issue #826): real loopback TCP open against an axum
        // server that is already listening before this line, so this
        // normally resolves in low ms. Only job is to fail "never connects"
        // rather than hang, not a timing claim.
        let mut stream = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream timed out")
            .expect("open_stream");

        let mut dialer = TsHttpDialer;
        let session = dialer.dial().expect("dial is infallible and does no I/O");
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        assert!(matches!(driver.health(), HealthState::Establishing));

        let mut cursor = None;
        let mut saw_sample = false;
        let mut ended = false;
        for i in 0..200u64 {
            // HANG GUARD (issue #826): per-iteration read timeout within
            // a 200-iteration loopback test. Loopback TCP resolves in ~ms;
            // this only prevents fast spinning on an empty stream, not a
            // timing claim.
            match recv_and_feed(
                &mut stream,
                &mut driver,
                Duration::from_secs(60),
                Timestamp::from_nanos(i),
            )
            .await
            {
                Ok(StreamStatus::Fed) => {}
                Ok(StreamStatus::Ended) => {
                    driver.finish();
                    ended = true;
                }
                Err(_) => break,
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while let Some(item) = c.poll() {
                    if matches!(item, media_plane::trunk::SampleCursorItem::Timed { .. }) {
                        saw_sample = true;
                    }
                }
            }
            if ended {
                break;
            }
        }
        assert!(
            saw_sample,
            "expected at least one sample from the muxed TS stream over HTTP in the Trunk"
        );
        // The axum helper closes the body at end-of-stream, so this route
        // ends cleanly rather than failing — the `Ended` vs `Failed`
        // distinction step 3c made producible, now actually produced.
        assert!(ended, "the chunked server's body must end cleanly");
        assert!(
            matches!(driver.health(), HealthState::Ended),
            "a cleanly-ended HTTP body must be Ended, not Failed: {:?}",
            driver.health()
        );

        server.abort();
    }

    /// A `404` (or any non-2xx) must fail `connect`, not silently proceed as
    /// if a track set would eventually resolve.
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
        // HANG GUARD (issue #826): the axum 404 handler returns immediately,
        // so `open_stream` fails in ~ms on the HTTP response. Only job is
        // to fail "never fails" rather than hang, not a timing claim.
        let result = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream timed out");
        assert!(result.is_err(), "a 404 must fail open_stream()");

        server.abort();
    }

    /// Biting test (issue #663 P5, audit-ingest #3): a server that resolves
    /// the track set and then goes silent — never sends another chunk, never
    /// closes the connection — must fail `next_samples()` within the
    /// configured [`IngestTimeouts::read`], not hang forever (the exact
    /// wedged/half-open failure mode a "no read timeout" `TsHttpSession`
    /// used to hang on). A raw TCP listener (not the axum chunked helper
    /// above, which always closes at end-of-body) plays the "accepts then
    /// stalls mid-body" server: valid headers + the PMT-resolving TS bytes,
    /// promised via a `Content-Length` far larger than what's ever actually
    /// written, so the client's body stream genuinely waits for more.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_mid_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");

        let _server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await; // drain the request, unparsed
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {}\r\n\r\n",
                ts_bytes.len() * 10 // promise far more than will ever be sent
            );
            stream
                .write_all(header.as_bytes())
                .await
                .expect("write header");
            stream
                .write_all(&ts_bytes)
                .await
                .expect("write body prefix");
            // Go silent forever: never write again, never close — the
            // stalled/wedged-server failure mode.
            std::future::pending::<()>().await;
        });

        let route = TsHttpRoute::new("stalled", format!("http://{addr}/stream.ts")).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(100),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // HANG GUARD (issue #826): backstop around the whole `run_ts_http`
        // call. The test's semantic assertion is that the 100ms read timeout
        // causes `run_ts_http` to return on its own; this outer timeout only
        // exists to fail "still running after 60s" rather than hang CI, not
        // a timing claim. The `read_times_out` assertion against
        // `IngestTimeouts::read` is what proves the test's invariant, not
        // this ceiling.
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            run_ts_http(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect(
            "run_ts_http must return on its own via IngestTimeouts::read, \
             not hang until this test's own backstop timeout",
        );
        assert!(
            result.is_err(),
            "a server that goes silent mid-body must fail, not hang forever"
        );
    }

    // --- issue #663 "Finish client-side multi-scheme auth": Basic/Digest/
    // Bearer/wrong-creds against a real mock auth server ---

    const AUTH_USER: &str = "cam-user";
    const AUTH_PASS: &str = "cam-pass";
    const DIGEST_REALM: &str = "mock realm";
    const BEARER_TOKEN: &str = "ts-http-bearer-token";

    /// Drives a `TsHttpRoute` to open and pull every sample the server
    /// serves, returning the sample count — the common "auth worked, real
    /// media came out" assertion shared by the Basic/Digest/Bearer tests
    /// below.
    async fn connect_and_drain(route: TsHttpRoute) -> Result<usize> {
        // HANG GUARD (issue #826): loopback TCP open against a server that
        // is already listening; normally resolves in ~ms. Only job is to
        // fail "never connects" rather than hang, not a timing claim.
        let mut stream = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .map_err(|_| MultimuxError::Connect {
                reason: "open_stream timed out".into(),
            })??;
        let mut dialer = TsHttpDialer;
        let session = dialer.dial().expect("infallible");
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut cursor = None;
        let mut total = 0usize;
        for i in 0..200u64 {
            // HANG GUARD (issue #826): per-iteration read timeout within a
            // 200-iteration loopback test. Loopback TCP resolves in ~ms;
            // this only prevents fast spinning on an empty stream.
            match recv_and_feed(
                &mut stream,
                &mut driver,
                Duration::from_secs(60),
                Timestamp::from_nanos(i),
            )
            .await
            {
                Ok(StreamStatus::Fed) => {}
                Ok(StreamStatus::Ended) | Err(_) => break,
            }
            if cursor.is_none() {
                cursor = driver.trunk(ProgramId(0)).map(|t| t.subscribe());
            }
            if let Some(c) = cursor.as_mut() {
                while c.poll().is_some() {
                    total += 1;
                }
            }
        }
        Ok(total)
    }

    /// Basic (RFC 7617), credentials from URL userinfo: the server issues a
    /// Basic challenge, the route answers it via
    /// `source::http_auth::authenticated_get`'s retry path, and real samples
    /// come out.
    #[tokio::test]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }),
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);

        let route = TsHttpRoute::new("cam-basic", credentialed);
        let total = connect_and_drain(route)
            .await
            .expect("Basic auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Basic auth");

        server.abort();
    }

    /// Digest (RFC 7616), credentials from URL userinfo: the server issues a
    /// real Digest challenge (nonce/realm/qop=auth) via a real
    /// `broadcast_auth::Verifier` (`crate::testutil::require_auth`) that
    /// independently recomputes the expected response — a client that can't
    /// answer it gets nothing back, so this proves the route actually
    /// computed a correct Digest response via `broadcast-auth`, not just
    /// echoed something Digest-shaped.
    #[tokio::test]
    async fn digest_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
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

        let route = TsHttpRoute::new("cam-digest", credentialed);
        let total = connect_and_drain(route)
            .await
            .expect("Digest auth from URL userinfo must authenticate");
        assert!(total > 0, "expected real samples after Digest auth");

        server.abort();
    }

    /// Bearer (RFC 6750), config-supplied (the only way to supply one — it
    /// has no URL-userinfo form): `TsHttpRoute::with_auth` overrides the
    /// (bare, no-userinfo) connect URL.
    #[tokio::test]
    async fn bearer_auth_config_supplied_authenticates_and_pulls_samples() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
        let (url, server) = start_chunked_ts_server_with_auth(
            ts_bytes,
            Some(crate::testutil::MockAuthScheme::Bearer {
                token: BEARER_TOKEN.into(),
            }),
        )
        .await;

        let route =
            TsHttpRoute::new("cam-bearer", url).with_auth(Some(Credentials::bearer(BEARER_TOKEN)));
        let total = connect_and_drain(route)
            .await
            .expect("config-supplied Bearer token must authenticate");
        assert!(total > 0, "expected real samples after Bearer auth");

        server.abort();
    }

    /// Config-supplied auth takes precedence over URL userinfo: the URL
    /// carries a *wrong* password, but `TsHttpRoute::with_auth` supplies the
    /// correct one — connect must succeed on the config auth, proving
    /// `resolve_credentials` really overrides rather than merely falling
    /// back.
    #[tokio::test]
    async fn config_auth_overrides_wrong_url_userinfo() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
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
        let total = connect_and_drain(route)
            .await
            .expect("config auth must override the URL's wrong userinfo password");
        assert!(
            total > 0,
            "expected real samples via config-overridden auth"
        );

        server.abort();
    }

    /// Wrong credentials must fail `connect()` (stay `401`), not hang or
    /// silently proceed — the negative counterpart to the three tests above,
    /// proving they actually bite (a client answering with the wrong
    /// password gets rejected exactly like a client with none).
    #[tokio::test]
    async fn wrong_credentials_stay_401_and_connect_errors() {
        let ts_bytes = build_ts_bytes(1, 0xAB, 10);
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

        let route = TsHttpRoute::new("cam-wrong", wrong_creds);
        // HANG GUARD (issue #826): the mock server returns 401
        // immediately (no body to parse), so `open_stream` fails in ~ms.
        // Only job is to fail "never fails" rather than hang.
        let result = tokio::time::timeout(Duration::from_secs(60), open_stream(&route))
            .await
            .expect("open_stream must not hang against a persistent 401");
        assert!(
            result.is_err(),
            "wrong credentials must fail open_stream(), not silently proceed"
        );

        server.abort();
    }
}
