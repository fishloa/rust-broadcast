//! Several pull sessions running at once on the multi-thread runtime: each is its own task,
//! none blocks another, and every one publishes media.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use broadcast_common::Timestamp;
use media_plane::ingress::HandshakePolicy;
use media_plane::trunk::{SampleCursorItem, TrunkConfig};
use multimux::RouteHandle;
use multimux::source::ts_http::{TsHttpRoute, run_ts_http};
use tokio_util::sync::CancellationToken;

use crate::support::listener;
use crate::support::shared_it::nz;

const FIXTURE: &str = "hls/ts-hls/index0.ts";
const TS_CHUNK: usize = 7 * 188;
const RING: usize = 16;
const TIMED_RING: usize = 4096;
const PULLS: usize = 8;
const CONNECT: Duration = Duration::from_secs(5);
const READ: Duration = Duration::from_secs(5);
const CEILING: Duration = Duration::from_secs(60);
const TRUNK_WAIT: Duration = Duration::from_secs(5);

async fn origin(body: Vec<u8>) -> String {
    let app = Router::new().route(
        "/stream.ts",
        get(move || {
            let body = body.clone();
            async move {
                let chunks: Vec<Result<Vec<u8>, std::io::Error>> =
                    body.chunks(TS_CHUNK).map(|c| Ok(c.to_vec())).collect();
                (
                    [(header::CONTENT_TYPE, "video/mp2t")],
                    Body::from_stream(futures_util::stream::iter(chunks)),
                )
                    .into_response()
            }
        }),
    );
    let (addr, tcp) = listener::bind_tcp();
    tokio::spawn(async move {
        axum::serve(tcp, app).await.expect("axum server");
    });
    format!("http://{addr}/stream.ts")
}

async fn samples(route_handle: &RouteHandle) -> usize {
    let trunk = tokio::time::timeout(TRUNK_WAIT, route_handle.await_first_trunk())
        .await
        .expect("the pull must have published a trunk");
    let mut cursor = trunk.subscribe_from_backlog();
    let mut count = 0;
    while let Some(item) = cursor.poll() {
        if matches!(item, SampleCursorItem::Timed { .. }) {
            count += 1;
        }
    }
    count
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_ts_http_pulls_all_progress_on_the_multi_thread_runtime() {
    let body = test_fixtures::read(FIXTURE);
    let mut pulls = Vec::new();
    for i in 0..PULLS {
        let url = origin(body.clone()).await;
        let route = TsHttpRoute::new(format!("pull-{i}"), url).with_timeouts(
            broadcast_common::timeouts::Timeouts::default()
                .with_connect(CONNECT)
                .with_read_idle(READ),
        );
        let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
        let config = TrunkConfig::default()
            .with_timed_capacity(nz(TIMED_RING))
            .with_sparse_capacity(nz(RING))
            .with_segment_capacity(nz(RING))
            .with_event_capacity(nz(RING))
            .with_part_capacity(nz(RING));
        let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
        let handle = Arc::clone(&route_handle);
        let task = tokio::spawn(async move {
            run_ts_http(&route, config, handshake, &handle, CancellationToken::new()).await
        });
        pulls.push((task, route_handle));
    }
    for (task, route_handle) in pulls {
        let result = tokio::time::timeout(CEILING, task)
            .await
            .expect("a pull must finish inside the ceiling")
            .expect("the pull task must not panic");
        assert!(result.is_ok(), "{result:?}");
        assert!(samples(&route_handle).await > 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_a_pull_stops_its_session_promptly() {
    let (addr, tcp) = listener::bind_tcp();
    let (accepted_tx, accepted) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (_held, _) = tcp.accept().await.expect("accept");
        let _ = accepted_tx.send(());
        std::future::pending::<()>().await;
    });
    let route = TsHttpRoute::new("hung", format!("http://{addr}/stream.ts"));
    let route_handle = Arc::new(RouteHandle::new(4.0, 500, 4));
    let config = TrunkConfig::default()
        .with_timed_capacity(nz(TIMED_RING))
        .with_sparse_capacity(nz(RING))
        .with_segment_capacity(nz(RING))
        .with_event_capacity(nz(RING))
        .with_part_capacity(nz(RING));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let task =
        tokio::spawn(
            async move { run_ts_http(&route, config, handshake, &route_handle, token).await },
        );
    accepted.await.expect("the origin must see the connection");
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("a cancelled pull must stop promptly")
        .expect("the pull task must not panic");
    assert!(result.is_err(), "{result:?}");
}
