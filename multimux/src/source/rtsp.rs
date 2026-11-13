//! RTSP ingest source: DESCRIBE/SETUP/PLAY over interleaved TCP (or TLS for `rtsps://`),
//! driven by a relay-mode [`broadcast_client::Session`] through the shared session host
//! (`crate::source::client_host`).
//!
//! The session owns the protocol: the `rtsp-runtime` engine, Basic/Digest auth, redirects, the
//! keepalive schedule (from the `Session;timeout=` the SETUP reply declared, RFC 2326 §12.37),
//! interleaved RTP/RTCP depayload, Sender-Report A/V alignment and the `TEARDOWN`. The host opens
//! its TCP/TLS connection, writes what the session sends, and says goodbye on cancel.
//!
//! # `rtsps://` (TLS)
//!
//! The session only ever exchanges plaintext bytes; the host performs the TLS handshake
//! (trusting the public-CA `webpki-roots` bundle via
//! `rtsp_runtime::io::default_tls_client_config`). Gated behind this crate's `tls` feature
//! (default-on); with it disabled, an `rtsps://` route fails with a clear error naming the
//! missing feature, never falling back to an unencrypted socket (issue #804).

use std::convert::Infallible;

use broadcast_client::{Config, SourceKind};
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;

use crate::error::MultimuxError;
use crate::source::client_host::{EventIngest, SessionLabels, SessionStep, TlsHost};
use crate::source::pull_target::{PullKind, PullRoute};

/// An RTSP route: a name (for logging/metrics) plus its source URL.
pub type RtspRoute = PullRoute<RtspKind>;

/// The [`PullKind`] of a [`RtspRoute`].
#[derive(Debug, Clone, Copy)]
pub struct RtspKind;

impl PullKind for RtspKind {
    const ROUTE: &'static str = "RtspRoute";
    type Extra = ();
}

impl crate::source::pull_target::PullDrive for RtspKind {
    const SEGMENTED: bool = false;

    async fn drive(
        route: std::sync::Arc<crate::source::pull_target::PullRoute<Self>>,
        trunk_config: media_plane::trunk::TrunkConfig,
        handshake: media_plane::ingress::HandshakePolicy,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::Result<()> {
        Err(run_rtsp(&route, trunk_config, handshake, &route_handle, cancel).await)
    }
}

const WHAT: &str = "rtsp";
const DESCRIBE_PHASE: &str = "DESCRIBE";

/// Opens `route` in a relay-mode session and drives it until the connection ends or fails. Never
/// returns `Ok`: a stream that ends is reported as an error and the route supervisor reconnects.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2: after every fed
/// event `crate::source::report_driver_progress` flips it live and publishes each programme's
/// `Trunk`.
pub async fn run_rtsp(
    route: &RtspRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
) -> MultimuxError {
    run_rtsp_with(
        route,
        trunk_config,
        handshake,
        route_handle,
        cancel,
        TlsHost::default(),
    )
    .await
}

pub(crate) async fn run_rtsp_with(
    route: &RtspRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    cancel: tokio_util::sync::CancellationToken,
    tls: TlsHost,
) -> MultimuxError {
    let timeouts = route.target.timeouts;
    let mut config = Config::relay();
    config.kind = SourceKind::Rtsp;
    config.max_queued_bytes = crate::source::LIVE_EVENT_QUEUE_BYTES;
    config.ts_http = crate::source::ts_http::policy_config(timeouts);
    let step = SessionStep::open(
        &route.target.url,
        config,
        route.target.auth.clone(),
        SessionLabels::new(WHAT, timeouts).not_found_in_phase(DESCRIBE_PHASE),
        std::sync::Arc::clone(route_handle),
        false,
    );
    let mut step = match step {
        Ok(step) => step.with_tls(tls),
        Err(e) => return e,
    };
    let driver = IngestDriver::new(
        EventIngest::new(),
        media_plane::IngestConfig::new(handshake).with_trunk(trunk_config),
    );
    let mut progress = crate::source::DriverProgress::new();
    let mut sink = tokio::io::sink();
    crate::source::driver::run_ingest_scaffold(
        driver,
        route_handle,
        &mut progress,
        cancel,
        timeouts,
        crate::source::driver::DEFAULT_WRITE_TIMEOUT,
        &mut sink,
        &mut step,
        |never: Infallible| match never {},
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    fn trunk_config() -> TrunkConfig {
        let nz = |n: usize| std::num::NonZeroUsize::new(n).unwrap();
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(16))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(broadcast_common::Timestamp::from_nanos(u64::MAX))
    }

    #[test]
    fn rtsp_route_debug_redacts_credentials() {
        let route = RtspRoute::new("cam1", "rtsp://user:secretpass@host/s");
        let debug = format!("{route:?}");
        assert!(
            !debug.contains("secretpass"),
            "debug leaked password: {debug}"
        );
        assert!(debug.contains("***@host"), "debug: {debug}");
    }

    #[tokio::test]
    async fn a_bad_url_error_redacts_credentials() {
        let route = RtspRoute::new("bad", "rtsp://user:secretpass@bad host/s");
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let error = run_rtsp(
            &route,
            trunk_config(),
            handshake(),
            &route_handle,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
        let msg = error.to_string();
        assert!(!msg.contains("user"), "error leaked username: {msg}");
        assert!(!msg.contains("secretpass"), "error leaked password: {msg}");
    }

    /// Builds a real RTSP/1.0 response (RFC 2326 §6): status line + headers
    /// (+ body, with a matching `Content-Length`) — hand-written wire text,
    /// but to the real grammar `rtsp_types::Message::parse` (driven via
    /// `ClientSession::handle_data`) actually parses, not a shortcut.
    fn rtsp_response(cseq: u32, status: &str, extra_headers: &str, body: &str) -> Vec<u8> {
        let content_length = if body.is_empty() {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        format!("RTSP/1.0 {status}\r\nCSeq: {cseq}\r\n{extra_headers}{content_length}\r\n{body}")
            .into_bytes()
    }

    /// A minimal, valid single-track SDP body (RFC 4566), the same fixture
    /// the pre-5a RTP/RTSP tests used.
    fn sdp_body() -> String {
        let sprop = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
        format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 127.0.0.1\r\n\
             s=-\r\n\
             t=0 0\r\n\
             m=video 0 RTP/AVP 96\r\n\
             a=control:trackID=1\r\n\
             a=rtpmap:96 H264/90000\r\n\
             a=fmtp:96 packetization-mode=1;sprop-parameter-sets={sprop}\r\n"
        )
    }

    /// Fail-safe property: an `rtsps://` route must never fall back to an
    /// unencrypted socket, even though establishing TLS against this
    /// deliberately-plain listener will fail. Points a real `rtsps://` route
    /// at a plain (non-TLS) loopback listener, through the real
    /// [`run_rtsp`] entry point (not a lower-level helper), and asserts on
    /// what the listener actually received, rather than trusting the code
    /// path: a genuine `rustls` `ClientHello` is opaque binary starting with
    /// TLS record type `0x16`, never the ASCII text of an RTSP request line —
    /// confirmed against the actual bytes captured below, not merely a
    /// keyword search.
    ///
    /// MUTATION VERIFIED: changing `run_rtsp`'s dispatch (`if is_tls {
    /// connect_tls(...) } else { connect_plain(...) }`) to call
    /// `connect_plain(&addr)` unconditionally (reintroducing a scheme-blind
    /// fallback) makes this fail at `assert_eq!(received[0], 0x16, ...)` with
    /// `left: 0x44, right: 0x16` — `0x44` is ASCII `'D'`, the first byte of
    /// the plaintext `DESCRIBE ...` request line the listener then receives
    /// instead of a TLS `ClientHello`.
    #[tokio::test]
    async fn rtsps_route_never_falls_back_to_plain_socket() {
        const TLS_HANDSHAKE_RECORD_TYPE: u8 = 0x16;
        const TLS_MAJOR_VERSION: u8 = 0x03;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 256];
            // HANG GUARD (issue #826): best-effort read to capture what the
            // client sent before giving up. The test's semantic assertions
            // (TLS record type and version bytes) are on the content of
            // whatever was written, not on how fast it arrives; 3s was
            // generous for loopback but under load even a 1-byte write can
            // be delayed by local scheduling. Raised to 60s since this is
            // purely a "did the client write anything at all" check, not a
            // timing claim.
            let n = tokio::time::timeout(std::time::Duration::from_secs(60), sock.read(&mut buf))
                .await
                .unwrap_or(Ok(0))
                .unwrap_or(0);
            buf[..n].to_vec()
        });

        // A short connect timeout on this *route instance* (via the existing
        // public `with_timeouts` builder) so the test doesn't wait out the
        // production `DEFAULT_CONNECT_TIMEOUT` default — this changes no
        // production timeout, only this one test route's own value.
        let route = RtspRoute::new("fail-safe", format!("rtsps://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_millis(500),
                read: std::time::Duration::from_millis(500),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));

        // HANG GUARD (issue #826): the test's semantic assertion is that
        // the rtsps:// client sends a real TLS ClientHello on the wire
        // (the TLS record-type and version-byte checks below). The TLS
        // handshake against a plaintext listener fails instantly — the
        // client sends its ClientHello, the plain TCP server reads it and
        // closes the connection, and `connect_tls` returns an error
        // instantly on the EOF. Verified: with connect timeout inflated
        // to 30s, the test still passes in <100ms (TLS handshake failure
        // is immediate, not timeout-driven). Raised to 60s for load
        // tolerance since this is not a timing claim — only job is to
        // fail a deadlock rather than hang CI.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_rtsp must not hang against a non-TLS listener");
        assert!(
            matches!(result, MultimuxError::Connect { .. }),
            "an rtsps:// connect against a plaintext listener must fail cleanly: {result:?}"
        );

        let received = server.await.expect("server task");
        assert!(
            received.len() >= 2,
            "the client must have sent real TLS handshake bytes before giving up: {received:?}"
        );
        assert_eq!(
            received[0], TLS_HANDSHAKE_RECORD_TYPE,
            "the first byte on the wire must be a TLS handshake record (0x16) — a genuine \
             ClientHello — never the ASCII first byte of a plaintext RTSP request line: \
             {received:?}"
        );
        assert_eq!(
            received[1], TLS_MAJOR_VERSION,
            "the TLS record's major version byte must be 0x03 (TLS 1.x): {received:?}"
        );
    }

    // --- issue #1083 W8: permanent-failure classification + keepalive ---

    const KEEPALIVE_METHOD: &str = "GET_PARAMETER";

    const PUBLIC_ALL: &str = "Public: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN, GET_PARAMETER\r\n";

    /// Serves one RTSP connection: reads each request's request line, and
    /// answers via `respond`. Returns every request line it saw, stopping
    /// once a keepalive (`GET_PARAMETER`, the session engine's liveness ping)
    /// arrives unless `answer_keepalives` asks for it to be answered and the
    /// connection served on.
    async fn serve_requests(
        sock: tokio::net::TcpStream,
        respond: impl FnMut(&str, u32) -> Option<Vec<u8>>,
    ) -> Vec<String> {
        serve_requests_answering_keepalives(sock, respond, false).await
    }

    async fn serve_requests_answering_keepalives(
        mut sock: tokio::net::TcpStream,
        mut respond: impl FnMut(&str, u32) -> Option<Vec<u8>>,
        answer_keepalives: bool,
    ) -> Vec<String> {
        let mut seen = Vec::new();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = match tokio::time::timeout(
                std::time::Duration::from_secs(6),
                sock.read(&mut chunk),
            )
            .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => n,
                Ok(Err(_)) => break,
            };
            buf.extend_from_slice(&chunk[..n]);
            // Parse every complete request (terminated by CRLFCRLF, no body
            // here) out of the buffer.
            while let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                let req = buf.drain(..pos + 4).collect::<Vec<u8>>();
                let text = String::from_utf8_lossy(&req).to_string();
                let method = text.split_whitespace().next().unwrap_or("").to_string();
                let cseq: u32 = text
                    .lines()
                    .find_map(|l| l.strip_prefix("CSeq:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                seen.push(format!("{method} {cseq}"));
                if method == KEEPALIVE_METHOD && !answer_keepalives {
                    // Stop as soon as a keepalive is observed — the point of
                    // this test is that one arrives at all.
                    return seen;
                }
                if let Some(resp) = respond(&method, cseq)
                    && sock.write_all(&resp).await.is_err()
                {
                    return seen;
                }
                if method == "TEARDOWN" {
                    return seen;
                }
            }
        }
        seen
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    /// A `401` on DESCRIBE must surface as [`MultimuxError::Auth`] from the
    /// real [`run_rtsp`] entry point, so `origin::supervisor::is_auth_failure`
    /// can classify it as permanent rather than retrying a wrong password
    /// forever.
    ///
    /// Biting test: restore the pre-fix `MultimuxError::Connect { reason:
    /// format!("…{:?}", driver.health()) }` tail and the returned error is a
    /// `Connect`, failing the `matches!(..., MultimuxError::Auth { .. })`
    /// assertion.
    #[tokio::test]
    async fn rtsp_describe_401_surfaces_as_a_typed_auth_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            serve_requests(sock, |_method, _cseq| {
                Some(rtsp_response(
                    1,
                    "401 Unauthorized",
                    "WWW-Authenticate: Basic realm=test\r\n",
                    "",
                ))
            })
            .await
        });

        let route = RtspRoute::new("auth", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_secs(5),
                read: std::time::Duration::from_secs(5),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_rtsp must not hang");

        assert!(
            matches!(err, MultimuxError::Auth { .. }),
            "a 401 on DESCRIBE must surface as the typed Auth error the supervisor  \
            classifies as permanent, got {err:?}"
        );
        let _ = server.await;
    }

    /// A non-success DESCRIBE answer other than 401/403/404 is reported as the protocol error
    /// naming the method and the status the server sent, promptly, so an operator sees the
    /// server's answer instead of a media timeout.
    #[tokio::test]
    async fn rtsp_describe_503_surfaces_the_status_promptly() {
        const READ: Duration = Duration::from_secs(30);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            serve_requests(sock, |_method, cseq| {
                Some(rtsp_response(cseq, "503 Service Unavailable", "", ""))
            })
            .await
        });
        let route = RtspRoute::new("unavailable", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: Duration::from_secs(5),
                read: READ,
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let started = std::time::Instant::now();
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("the refusal must end the run long before the media timeout");
        assert!(
            matches!(
                &err,
                MultimuxError::Protocol { phase, reason }
                    if *phase == "DESCRIBE" && reason == "non-success status Service Unavailable"
            ),
            "got {err:?}"
        );
        assert!(
            !crate::origin::supervisor::is_permanent_describe_not_found(&err),
            "a transient refusal is not permanent"
        );
        assert!(started.elapsed() < READ);
        let _ = server.await;
    }

    /// A `404` on DESCRIBE (a wrong URL path) must surface as the exact
    /// `MultimuxError::Protocol { phase: "DESCRIBE", reason: "non-success
    /// status Not Found" }` that `is_permanent_describe_not_found` matches.
    #[tokio::test]
    async fn rtsp_describe_404_surfaces_as_the_permanent_protocol_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            serve_requests(sock, |_method, _cseq| {
                Some(rtsp_response(1, "404 Not Found", "", ""))
            })
            .await
        });

        let route = RtspRoute::new("notfound", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_secs(5),
                read: std::time::Duration::from_secs(5),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let err = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_rtsp must not hang");

        assert!(
            matches!(
                &err,
                MultimuxError::Protocol { phase, reason }
                    if *phase == "DESCRIBE" && reason == "non-success status Not Found"
            ),
            "a 404 on DESCRIBE must surface the exact Protocol error the supervisor  \
            classifies as permanent, got {err:?}"
        );
        // The classification the supervisor actually applies.
        assert!(
            crate::origin::supervisor::is_permanent_describe_not_found(&err),
            "the supervisor's own predicate must match this error"
        );
        let _ = server.await;
    }

    /// With a server that declares `Session; timeout=2`, an idle `Live`
    /// session must send the keepalive `rtsp-runtime`'s session engine
    /// schedules before the timeout elapses (RFC 2326 §12.37), rather than
    /// sitting silently until the server tears the session down. This server
    /// answers the session-level `OPTIONS` with a `Public` header listing
    /// `GET_PARAMETER`, so that is the method its interleaved keepalive uses.
    ///
    /// Biting test: make `on_deadline` a no-op (its pre-fix body) and the
    /// server never sees the keepalive, failing the assertion below.
    #[tokio::test]
    async fn rtsp_sends_a_keepalive_when_the_server_declares_a_timeout() {
        let seen_keepalive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let keepalive_flag = seen_keepalive.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");

        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            let sdp = sdp_body();
            serve_requests_answering_keepalives(
                sock,
                move |method, cseq| {
                    if method == KEEPALIVE_METHOD {
                        seen_keepalive.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    let session = "Session: 12345678; timeout=2\r\n";
                    let resp = match method {
                        "DESCRIBE" => {
                            rtsp_response(cseq, "200 OK", "Content-Type: application/sdp\r\n", &sdp)
                        }
                        "SETUP" => rtsp_response(
                            cseq,
                            "200 OK",
                            "Session: 12345678; timeout=2\r\nTransport: RTP/AVP/TCP;interleaved=0-1\r\n",
                            "",
                        ),
                        "PLAY" | KEEPALIVE_METHOD => rtsp_response(cseq, "200 OK", session, ""),
                    "OPTIONS" => rtsp_response(cseq, "200 OK", PUBLIC_ALL, ""),
                        _ => return None,
                    };
                    Some(resp)
                },
                true,
            )
            .await
        });

        let route = RtspRoute::new("keepalive", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: std::time::Duration::from_secs(5),
                read: std::time::Duration::from_secs(5),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        // The route only ends when the server closes; run it concurrently and
        // observe what the server saw.
        let run = tokio::spawn(async move {
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
        });
        let observed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            while !keepalive_flag.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        run.abort();
        server.abort();
        observed.expect(
            "the client must have sent a GET_PARAMETER keepalive within the 2s session timeout",
        );
    }

    /// Cancelling a playing route sends one `TEARDOWN` (RFC 2326 §10.7) before the connection is
    /// dropped, so the server releases its session instead of waiting for its timeout.
    #[tokio::test]
    async fn cancelling_a_playing_route_sends_a_teardown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let (played_tx, played) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            let sdp = sdp_body();
            let mut played_tx = Some(played_tx);
            serve_requests(sock, move |method, cseq| {
                let resp = match method {
                    "DESCRIBE" => {
                        rtsp_response(cseq, "200 OK", "Content-Type: application/sdp\r\n", &sdp)
                    }
                    "SETUP" => rtsp_response(
                        cseq,
                        "200 OK",
                        "Session: 12345678\r\nTransport: RTP/AVP/TCP;interleaved=0-1\r\n",
                        "",
                    ),
                    "PLAY" => {
                        if let Some(tx) = played_tx.take() {
                            let _ = tx.send(());
                        }
                        rtsp_response(cseq, "200 OK", "Session: 12345678\r\n", "")
                    }
                    "TEARDOWN" => rtsp_response(cseq, "200 OK", "Session: 12345678\r\n", ""),
                    _ => return None,
                };
                Some(resp)
            })
            .await
        });

        let route = RtspRoute::new("teardown", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_secs(30),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let cancel = tokio_util::sync::CancellationToken::new();
        let run = tokio::spawn({
            let cancel = cancel.clone();
            async move { run_rtsp(&route, trunk_config(), handshake(), &route_handle, cancel).await }
        });
        tokio::time::timeout(Duration::from_secs(30), played)
            .await
            .expect("the server must see PLAY")
            .expect("PLAY signal");
        cancel.cancel();
        let ended = tokio::time::timeout(Duration::from_secs(30), run)
            .await
            .expect("a cancelled route must stop")
            .expect("run task");
        assert!(matches!(ended, MultimuxError::Connect { .. }), "{ended:?}");
        let seen = tokio::time::timeout(Duration::from_secs(30), server)
            .await
            .expect("the server must see the TEARDOWN and finish")
            .expect("server task");
        assert_eq!(
            seen.iter().filter(|r| r.starts_with("TEARDOWN")).count(),
            1,
            "exactly one TEARDOWN: {seen:?}"
        );
    }

    #[cfg(feature = "tls")]
    mod tls_tests {
        use super::*;
        use std::sync::Arc;
        use tokio::net::{TcpListener, TcpStream};

        fn cert_der() -> Vec<u8> {
            test_fixtures::read("tls/localhost-cert.der")
        }
        fn key_der() -> Vec<u8> {
            test_fixtures::read("tls/localhost-key.der")
        }

        const HANG_GUARD: Duration = Duration::from_secs(60);

        fn tls_provider() -> Arc<rustls::crypto::CryptoProvider> {
            Arc::new(rustls::crypto::aws_lc_rs::default_provider())
        }

        fn server_config() -> rustls::ServerConfig {
            use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
            let certs = vec![CertificateDer::from(cert_der())];
            let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der()));
            rustls::ServerConfig::builder_with_provider(tls_provider())
                .with_safe_default_protocol_versions()
                .expect("aws-lc-rs provider supports the safe default protocol versions")
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .expect("server config")
        }

        fn client_config_trusting_fixture() -> rustls::ClientConfig {
            use rustls::pki_types::CertificateDer;
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(CertificateDer::from(cert_der()))
                .expect("add self-signed root");
            rustls::ClientConfig::builder_with_provider(tls_provider())
                .with_safe_default_protocol_versions()
                .expect("aws-lc-rs provider supports the safe default protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth()
        }

        async fn read_request_boundary<S: AsyncRead + Unpin>(sock: &mut S) {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    return;
                }
                let n = sock.read(&mut chunk).await.expect("read request bytes");
                assert!(n > 0, "peer closed before a full request arrived");
                buf.extend_from_slice(&chunk[..n]);
            }
        }

        async fn write_response<S: AsyncWrite + Unpin>(sock: &mut S, bytes: &[u8]) {
            sock.write_all(bytes).await.expect("write response");
            sock.flush().await.expect("flush response");
        }

        fn h264_rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
            const PT_H264_DYNAMIC: u8 = 96;
            const SSRC: u32 = 0xCAFE_BABE;
            let wire = rtp_packet::RtpPacket {
                marker,
                payload_type: PT_H264_DYNAMIC,
                sequence_number: seq,
                timestamp,
                ssrc: SSRC,
                csrc: Vec::new(),
                extension: None,
                padding: None,
                payload: nal,
            };
            let mut pkt = vec![0u8; broadcast_common::Serialize::serialized_len(&wire)];
            broadcast_common::Serialize::serialize_into(&wire, &mut pkt).expect("serialize RTP");
            pkt
        }

        async fn serve_one_tls_session(tcp: TcpStream) {
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config()));
            let mut tls = acceptor.accept(tcp).await.expect("TLS handshake (server)");

            read_request_boundary(&mut tls).await;
            let sdp = sdp_body();
            let describe_resp = format!(
                "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
                sdp.len()
            );
            write_response(&mut tls, describe_resp.as_bytes()).await;

            read_request_boundary(&mut tls).await;
            let setup_resp = "RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: TLSTEST\r\n\
                               Transport: RTP/AVP/TCP;interleaved=0-1\r\n\r\n";
            write_response(&mut tls, setup_resp.as_bytes()).await;

            read_request_boundary(&mut tls).await;
            let play_resp = "RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: TLSTEST\r\n\r\n";
            write_response(&mut tls, play_resp.as_bytes()).await;

            let idr = [0x65u8, 0xAA, 0xBB];
            let non1 = [0x41u8, 0xAA, 0xBB];
            let non2 = [0x41u8, 0xCC, 0xDD];
            let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
            for (i, (ts, nal)) in aus.into_iter().enumerate() {
                let pkt = h264_rtp_packet(1 + i as u16, ts, true, nal);
                let frame = rtsp_runtime::interleaved::InterleavedFrame::new(0, pkt)
                    .to_bytes()
                    .expect("serialize interleaved frame");
                write_response(&mut tls, &frame).await;
            }
            let mut sink = [0u8; 1024];
            while matches!(tls.read(&mut sink).await, Ok(n) if n > 0) {}
        }

        /// A genuine `tokio_rustls` handshake against a real self-signed-cert loopback server,
        /// then a real DESCRIBE -> SETUP -> PLAY -> depayload exchange over the encrypted
        /// socket: TLS is negotiated (a plaintext stream could never complete the handshake
        /// against a TLS-only server) and real media reaches the Trunk.
        #[tokio::test]
        async fn rtsps_route_negotiates_real_tls_and_streams() {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind loopback");
            let addr = listener.local_addr().expect("local addr");
            let server = tokio::spawn(async move {
                let (tcp, _) = listener.accept().await.expect("accept");
                serve_one_tls_session(tcp).await;
            });

            let route = RtspRoute::new("tls", format!("rtsps://localhost:{}/stream", addr.port()))
                .with_timeouts(crate::source::IngestTimeouts {
                    connect: HANG_GUARD,
                    read: HANG_GUARD,
                });
            let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
            let cancel = tokio_util::sync::CancellationToken::new();
            let run = tokio::spawn({
                let route_handle = std::sync::Arc::clone(&route_handle);
                let cancel = cancel.clone();
                async move {
                    run_rtsp_with(
                        &route,
                        trunk_config(),
                        handshake(),
                        &route_handle,
                        cancel,
                        TlsHost {
                            config: Some(client_config_trusting_fixture()),
                        },
                    )
                    .await
                }
            });

            let trunk = tokio::time::timeout(HANG_GUARD, route_handle.await_first_trunk())
                .await
                .expect("DESCRIBE/SETUP/PLAY must complete over the TLS transport");
            let mut cursor = trunk.subscribe_from_backlog();
            let first = tokio::time::timeout(HANG_GUARD, async {
                loop {
                    if let Some(media_plane::trunk::SampleCursorItem::Timed { track_id, sample }) =
                        cursor.poll()
                    {
                        return (track_id.value(), sample.flags.is_sync);
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("a depayloaded sample must reach the Trunk over TLS");
            assert_eq!(first.0, 1, "the sole video track routes to track id 1");
            assert!(first.1, "the first access unit was the IDR");
            cancel.cancel();
            let _ = tokio::time::timeout(HANG_GUARD, run).await;
            server.abort();
        }
    }

    /// A hostile `Session;timeout` value must not overflow the keepalive
    /// deadline, and a tiny one must not produce sub-second `OPTIONS` spam —
    /// both are the engine's floor (`MIN_KEEPALIVE_INTERVAL`), so this stage
    /// inherits them by driving `poll_timeout`/`handle_timeout` at all.
    ///
    /// Biting test: build the deadline as `Instant + timeout/2` without the
    /// engine's floor/`checked_add` and the `timeout=18446744073709551615`
    /// case panics instead of scheduling a keepalive.
    #[tokio::test]
    async fn hostile_session_timeouts_do_not_overflow_or_spam() {
        for declared in ["18446744073709551615", "0", "1", "60"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind loopback");
            let addr = listener.local_addr().expect("addr");
            let header = format!("Session: 12345678; timeout={declared}\r\n");
            let server = tokio::spawn(async move {
                let (sock, _) = listener.accept().await.expect("accept");
                let sdp = sdp_body();
                serve_requests(sock, move |method, cseq| {
                    let resp = match method {
                        "DESCRIBE" => {
                            rtsp_response(cseq, "200 OK", "Content-Type: application/sdp\r\n", &sdp)
                        }
                        "SETUP" => rtsp_response(
                            cseq,
                            "200 OK",
                            &format!("{header}Transport: RTP/AVP/TCP;interleaved=0-1\r\n"),
                            "",
                        ),
                        "PLAY" | KEEPALIVE_METHOD => rtsp_response(cseq, "200 OK", &header, ""),
                        _ => return None,
                    };
                    Some(resp)
                })
                .await
            });
            let route = RtspRoute::new("hostile", format!("rtsp://{addr}/stream")).with_timeouts(
                crate::source::IngestTimeouts {
                    connect: std::time::Duration::from_secs(5),
                    read: std::time::Duration::from_secs(5),
                },
            );
            let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
            let run = tokio::spawn(async move {
                run_rtsp(
                    &route,
                    trunk_config(),
                    handshake(),
                    &route_handle,
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
            });
            let observed = tokio::time::timeout(std::time::Duration::from_secs(20), server)
                .await
                .expect("the server must observe requests within the hang guard")
                .expect("server task");
            run.abort();
            assert!(
                observed.iter().any(|r| r.starts_with("PLAY")),
                "timeout={declared}: the session must reach PLAY, saw {observed:?}"
            );
        }
    }

    /// A server that completes the handshake and answers keepalives but
    /// sends no media must fail with the read-timeout error — a keepalive
    /// must not keep resetting the stall detector (issue #1083, item 1).
    ///
    /// Biting test: restore the old `Err(_) if wait < read_timeout =>
    /// continue` and this never returns the read-timeout error.
    #[tokio::test]
    async fn a_media_silent_session_fails_on_the_read_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");

        // A 300 ms read timeout with no declared session timeout (so the
        // keepalive default of 30 s never fires within the test): the read
        // timeout is what must end the session.
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            let sdp = sdp_body();
            serve_requests(sock, move |method, cseq| {
                let resp = match method {
                    "DESCRIBE" => {
                        rtsp_response(cseq, "200 OK", "Content-Type: application/sdp\r\n", &sdp)
                    }
                    "SETUP" => rtsp_response(
                        cseq,
                        "200 OK",
                        "Session: 12345678\r\nTransport: RTP/AVP/TCP;interleaved=0-1\r\n",
                        "",
                    ),
                    "PLAY" => rtsp_response(cseq, "200 OK", "Session: 12345678\r\n", ""),
                    // Keepalives are genuinely unanswered: this server made
                    // no `OPTIONS` exchange at all (no `Public` header), so
                    // its keepalive is `OPTIONS`, and a reply would carry the
                    // session id of a session it never declared.
                    "OPTIONS" | KEEPALIVE_METHOD => return None,
                    _ => return None,
                };
                Some(resp)
            })
            .await
        });

        let route = RtspRoute::new("stall", format!("rtsp://{addr}/stream")).with_timeouts(
            crate::source::IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(300),
            },
        );
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
        let err = tokio::time::timeout(
            Duration::from_secs(10),
            run_rtsp(
                &route,
                trunk_config(),
                handshake(),
                &route_handle,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
        .expect("run_rtsp must return (not hang) once the read timeout elapses");

        assert!(
            matches!(&err, MultimuxError::Connect { reason }
                if reason.contains("no data within")),
            "a media-silent session must fail with the read-timeout error, got {err:?}"
        );
        server.abort();
    }
}
