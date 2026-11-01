//! TS-over-HTTP ingest through the real `run_ts_http` drive loop, pinning the
//! behaviour of the shared `tshttp-runtime` policy core: finite bodies end
//! cleanly, a mid-body stall fails with the stall reason, and 3xx responses
//! are followed. Media is the committed `ts-hls` fixture over loopback HTTP.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::source::IngestTimeouts;
use multimux::source::ts_http::{TsHttpRoute, run_ts_http};
use multimux::{MultimuxError, RouteHandle};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::support::listener;

const FIXTURE: &str = "../hls-runtime/tests/fixtures/ts-hls/index0.ts";
const TS_CHUNK: usize = 7 * 188;
const TIMED_RING: usize = 4096;
const RING: usize = 16;
const CONNECT: Duration = Duration::from_secs(5);
const STALL_READ: Duration = Duration::from_secs(1);
const RUN_CEILING: Duration = Duration::from_secs(30);
const STALL_CEILING: Duration = Duration::from_secs(15);
const MISSING_PROMISE_BYTES: usize = 1_000_000;
const TRUNK_WAIT: Duration = Duration::from_secs(5);

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn fixture() -> Vec<u8> {
    test_fixtures::read(FIXTURE)
}

fn timeouts(read: Duration) -> IngestTimeouts {
    IngestTimeouts {
        connect: CONNECT,
        read,
    }
}

async fn run(route: &TsHttpRoute) -> (Result<(), MultimuxError>, Arc<RouteHandle>) {
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::new(nz(TIMED_RING), nz(RING), nz(RING), nz(RING), nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let result = tokio::time::timeout(
        RUN_CEILING,
        run_ts_http(
            route,
            config,
            handshake,
            &route_handle,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("run_ts_http must finish inside the ceiling");
    (result, route_handle)
}

async fn sample_count(route_handle: &RouteHandle) -> usize {
    let trunk = tokio::time::timeout(TRUNK_WAIT, route_handle.await_first_trunk())
        .await
        .expect("the ingest must have published a trunk");
    let mut cursor = trunk.subscribe_from_backlog();
    let mut count = 0;
    while let Some(item) = cursor.poll() {
        if matches!(item, SampleCursorItem::Timed { .. }) {
            count += 1;
        }
    }
    count
}

async fn spawn(app: Router) -> String {
    let (addr, tcp) = listener::bind_tcp();
    tokio::spawn(async move {
        axum::serve(tcp, app).await.expect("axum server");
    });
    format!("http://{addr}")
}

fn stream_body(bytes: Vec<u8>) -> Body {
    let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
        bytes.chunks(TS_CHUNK).map(|c| Ok(c.to_vec())).collect();
    Body::from_stream(futures_util::stream::iter(chunks))
}

async fn finite_server(declared: usize, body: Vec<u8>) -> String {
    let app = Router::new().route(
        "/stream.ts",
        get(move || {
            let body = body.clone();
            async move {
                (
                    [
                        (header::CONTENT_TYPE, "video/mp2t".to_string()),
                        (header::CONTENT_LENGTH, declared.to_string()),
                    ],
                    stream_body(body),
                )
                    .into_response()
            }
        }),
    );
    format!("{}/stream.ts", spawn(app).await)
}

async fn raw_server(declared: usize, prefix: Vec<u8>, then_close: bool) -> String {
    let (addr, tcp) = listener::bind_tcp();
    tokio::spawn(async move {
        let (mut stream, _) = tcp.accept().await.expect("accept");
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nContent-Length: {declared}\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).await.expect("head");
        stream.write_all(&prefix).await.expect("body prefix");
        if then_close {
            drop(stream);
        } else {
            std::future::pending::<()>().await;
        }
    });
    format!("http://{addr}/stream.ts")
}

#[tokio::test]
async fn body_of_declared_length_ends_cleanly_with_samples() {
    let body = fixture();
    let url = finite_server(body.len(), body).await;
    let route = TsHttpRoute::new("finite", url).with_timeouts(timeouts(STALL_READ));
    let (result, route_handle) = run(&route).await;
    assert!(
        result.is_ok(),
        "a fully delivered finite body is a clean end: {result:?}"
    );
    assert!(sample_count(&route_handle).await > 0);
}

#[tokio::test]
async fn body_cut_short_of_declared_length_fails() {
    let body = fixture();
    let url = raw_server(body.len() + MISSING_PROMISE_BYTES, body, true).await;
    let route = TsHttpRoute::new("truncated", url).with_timeouts(timeouts(STALL_READ));
    let (result, _) = run(&route).await;
    assert!(
        result.is_err(),
        "a body that ends before its Content-Length must not be reported as ended"
    );
}

#[tokio::test]
async fn mid_body_stall_fails_with_the_stall_reason() {
    let body = fixture();
    let url = raw_server(body.len() + MISSING_PROMISE_BYTES, body, false).await;
    let route = TsHttpRoute::new("stalled", url).with_timeouts(timeouts(STALL_READ));
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::new(nz(TIMED_RING), nz(RING), nz(RING), nz(RING), nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        STALL_CEILING,
        run_ts_http(
            &route,
            config,
            handshake,
            &route_handle,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("a stalled body must fail inside the ceiling, not hang");
    match result {
        Err(MultimuxError::Connect { reason }) => {
            assert!(reason.contains("no data within"), "{reason}");
        }
        other => panic!("expected a stall failure, got {other:?}"),
    }
    assert!(
        started.elapsed() >= STALL_READ,
        "failed before the configured stall"
    );
    assert!(
        sample_count(&route_handle).await > 0,
        "media before the stall was published"
    );
}

#[tokio::test]
async fn redirect_is_followed_to_the_stream() {
    let body = fixture();
    let declared = body.len();
    let app = Router::new()
        .route(
            "/live.ts",
            get(|| async { Redirect::temporary("/stream.ts") }),
        )
        .route(
            "/stream.ts",
            get(move || {
                let body = body.clone();
                async move {
                    (
                        [
                            (header::CONTENT_TYPE, "video/mp2t".to_string()),
                            (header::CONTENT_LENGTH, declared.to_string()),
                        ],
                        stream_body(body),
                    )
                        .into_response()
                }
            }),
        );
    let url = format!("{}/live.ts", spawn(app).await);
    let route = TsHttpRoute::new("redirected", url).with_timeouts(timeouts(STALL_READ));
    let (result, route_handle) = run(&route).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(sample_count(&route_handle).await > 0);
}

#[tokio::test]
async fn endless_redirects_fail_instead_of_looping() {
    async fn hop() -> Response {
        Redirect::temporary("/hop").into_response()
    }
    let app = Router::new().route("/hop", get(hop));
    let url = format!("{}/hop", spawn(app).await);
    let route = TsHttpRoute::new("loop", url).with_timeouts(timeouts(STALL_READ));
    let (result, _) = run(&route).await;
    assert!(result.is_err());
}
