//! The `media-doctor watch` metrics HTTP server (de-hand-roll W1-P, SP2.3).
//!
//! hyper does all HTTP framing; this module only decides *who may connect and
//! for how long* — the policy the old thread-per-connection server enforced
//! (audit MD-W9): a concurrency cap (`max_conns`; over-cap peers are closed at accept),
//! a header-read timeout and a total per-connection deadline (`io_timeout`)
//! so a dribbling or idle peer cannot hold a connection, and a
//! `CancellationToken` that stops the accept loop and drops the listener.
//!
//! Tasks are owned by a [`TaskTracker`]; `serve` returns only after every
//! connection task has ended.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::WatchState;

/// Prometheus text exposition content type (format 0.0.4), as the old server sent.
const EXPOSITION_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// Pause after a failed `accept()` (EMFILE under a connection storm, a peer that
/// aborted between connect and accept) so a persistent error cannot spin the loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(50);

/// Default concurrent-connection cap (matches the old CLI default).
pub const DEFAULT_MAX_CONNS: usize = 32;
/// Default total per-connection deadline (matches the old CLI default).
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_millis(5_000);

/// Server limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetricsServerConfig {
    /// Connections served with the metrics body at once; further peers are
    /// closed at accept.
    pub max_conns: usize,
    /// Both the header-read timeout and the total deadline per connection.
    pub io_timeout: Duration,
}

impl MetricsServerConfig {
    /// A config with the given connection cap and per-connection deadline.
    #[must_use]
    pub const fn new(max_conns: usize, io_timeout: Duration) -> Self {
        Self {
            max_conns,
            io_timeout,
        }
    }
}

impl Default for MetricsServerConfig {
    fn default() -> Self {
        Self {
            max_conns: DEFAULT_MAX_CONNS,
            io_timeout: DEFAULT_IO_TIMEOUT,
        }
    }
}

/// Renders [`WatchState`] into the shared body at most once per `interval` of
/// the caller's clock, so the ingest loop pays for exposition at the scrape
/// scale, not the datagram scale.
pub struct MetricsPublisher {
    tx: watch::Sender<Arc<str>>,
    interval: Duration,
    last: Option<Duration>,
}

/// Create the publisher and the receiver [`serve`] reads. The initial body is
/// rendered from `initial`, so the very first scrape already lists every family.
#[must_use]
pub fn channel(
    initial: &WatchState,
    interval: Duration,
) -> (MetricsPublisher, watch::Receiver<Arc<str>>) {
    let (tx, rx) = watch::channel(Arc::<str>::from(crate::render_metrics(initial)));
    (
        MetricsPublisher {
            tx,
            interval,
            last: None,
        },
        rx,
    )
}

impl MetricsPublisher {
    /// Publish if `interval` has elapsed since the last publish on `clock`.
    pub fn maybe_publish(&mut self, state: &WatchState, clock: Duration) -> bool {
        let due = self
            .last
            .is_none_or(|last| clock.saturating_sub(last) >= self.interval);
        if due {
            self.publish_now(state, clock);
        }
        due
    }

    /// Publish unconditionally (call when the feed goes quiet so the final
    /// figures become visible).
    pub fn publish_now(&mut self, state: &WatchState, clock: Duration) {
        self.tx
            .send_replace(Arc::<str>::from(crate::render_metrics(state)));
        self.last = Some(clock);
    }
}

fn respond(text: Arc<str>) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(text.as_bytes().to_vec())));
    r.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(EXPOSITION_CONTENT_TYPE),
    );
    r
}

/// Serve `GET /metrics` (and, this being a single-endpoint probe, any other
/// request) until `shutdown` is cancelled. See the module docs for the limits.
pub async fn serve(
    listener: TcpListener,
    metrics: watch::Receiver<Arc<str>>,
    config: MetricsServerConfig,
    shutdown: CancellationToken,
) {
    let permits = Arc::new(Semaphore::new(config.max_conns));
    let tracker = TaskTracker::new();
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let stream = match accepted {
            Ok((stream, _peer)) => stream,
            Err(e) => {
                eprintln!("media-doctor watch: metrics accept error: {e}");
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                continue;
            }
        };
        // Taken here, in accept order, so "the first N connections own the N
        // permits" is deterministic; released when the connection task ends.
        // Over the cap: close at once (drop the stream) — no task, no fd held.
        let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let metrics = metrics.clone();
        let token = shutdown.clone();
        tracker.spawn(async move {
            let service = service_fn(move |_req: Request<Incoming>| {
                let body = Arc::clone(&metrics.borrow());
                async move { Ok::<_, Infallible>(respond(body)) }
            });
            let mut http = http1::Builder::new();
            http.timer(TokioTimer::new())
                .header_read_timeout(config.io_timeout)
                .keep_alive(false);
            let conn = http.serve_connection(TokioIo::new(stream), service);
            tokio::pin!(conn);
            tokio::select! {
                result = tokio::time::timeout(config.io_timeout, conn.as_mut()) => {
                    if let Ok(Err(e)) = result {
                        eprintln!("media-doctor watch: metrics request error: {e}");
                    }
                }
                () = token.cancelled() => {
                    // Graceful: let a response already in flight finish (idle
                    // connections close at once), bounded by the deadline.
                    conn.as_mut().graceful_shutdown();
                    let _ = tokio::time::timeout(config.io_timeout, conn.as_mut()).await;
                }
            }
            drop(permit);
        });
    }
    tracker.close();
    tracker.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const REQ: &[u8] = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n";
    const BODY: &str = "# HELP media_doctor_packets_total x\n# TYPE media_doctor_packets_total counter\nmedia_doctor_packets_total 0\n";

    struct Server {
        addr: std::net::SocketAddr,
        token: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    }

    async fn start(cfg: MetricsServerConfig) -> (Server, tokio::sync::watch::Sender<Arc<str>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::watch::channel(Arc::<str>::from(BODY));
        let token = CancellationToken::new();
        let task = tokio::spawn(serve(listener, rx, cfg, token.clone()));
        (Server { addr, token, task }, tx)
    }

    async fn scrape(addr: std::net::SocketAddr) -> String {
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(REQ).await.unwrap();
        let mut out = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            s.read_to_string(&mut out),
        )
        .await
        .expect("scrape timed out")
        .unwrap();
        out
    }

    fn cfg(max_conns: usize, io_timeout_ms: u64) -> MetricsServerConfig {
        MetricsServerConfig {
            max_conns,
            io_timeout: std::time::Duration::from_millis(io_timeout_ms),
        }
    }

    /// Re-expresses `idle_client_does_not_block_a_second_scraper`.
    #[tokio::test]
    async fn idle_client_does_not_block_a_second_scraper() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let _idle = TcpStream::connect(srv.addr).await.unwrap();
        let body = scrape(srv.addr).await;
        assert!(body.starts_with("HTTP/1.1 200 OK"), "{body}");
        assert!(body.contains("media_doctor_packets_total 0"), "{body}");
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `repeated_scrapes_all_succeed_with_a_stalled_client_outstanding`.
    #[tokio::test]
    async fn repeated_scrapes_succeed_with_stalled_clients_outstanding() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let mut idle = Vec::new();
        for _ in 0..3 {
            idle.push(TcpStream::connect(srv.addr).await.unwrap());
        }
        for round in 0..3 {
            assert!(
                scrape(srv.addr).await.starts_with("HTTP/1.1 200 OK"),
                "round {round}"
            );
        }
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `connections_beyond_the_cap_are_refused` (the 503 and
    /// `refusal_response_uses_crlf_framing` are retired: over-cap sockets are
    /// now closed at accept, nothing is written). Bounded-pending proof: with
    /// the cap full, FLOOD over-cap connects are each closed promptly (EOF or
    /// reset well inside `io_timeout`), so none is parked holding a task/fd;
    /// the count of live server tasks never exceeds `max_conns`.
    #[tokio::test]
    async fn connections_beyond_the_cap_are_closed_promptly_and_do_not_accumulate() {
        const CAP: usize = 2;
        const FLOOD: usize = 200;
        let (srv, _tx) = start(cfg(CAP, 60_000)).await;
        // Connect sequentially: the accept loop admits in connect order, so
        // the first CAP connections own the permits (no probing needed).
        let mut held = Vec::new();
        for _ in 0..CAP {
            held.push(TcpStream::connect(srv.addr).await.unwrap());
        }
        let mut buf = [0u8; 16];
        for i in 0..FLOOD {
            let mut over = TcpStream::connect(srv.addr).await.unwrap();
            // Closed by the server: EOF or reset, within a bound far below the 60 s io_timeout.
            let r = tokio::time::timeout(std::time::Duration::from_secs(5), over.read(&mut buf))
                .await
                .unwrap_or_else(|_| panic!("over-cap socket {i} was held open, not closed"));
            assert!(
                matches!(r, Ok(0) | Err(_)),
                "over-cap socket {i} got data: {r:?}"
            );
        }
        // The held connections still own their permits and still work.
        for h in &mut held {
            h.write_all(REQ).await.unwrap();
        }
        for h in &mut held {
            let mut out = vec![0u8; 12];
            h.read_exact(&mut out).await.unwrap();
            assert_eq!(&out, b"HTTP/1.1 200");
        }
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `connection_flood_beyond_the_cap_still_serves_after_it_drains`.
    #[tokio::test]
    async fn flood_beyond_the_cap_drains_and_serving_resumes() {
        const CAP: usize = 4;
        let (srv, _tx) = start(cfg(CAP, 400)).await;
        let mut flood = Vec::new();
        for _ in 0..CAP * 6 {
            flood.push(TcpStream::connect(srv.addr).await.unwrap());
        }
        drop(flood);
        // Condition-wait with a bound (no fixed sleep): retry until a scrape is 200.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if scrape(srv.addr).await.starts_with("HTTP/1.1 200 OK") {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never recovered after the flood"
            );
            tokio::task::yield_now().await;
        }
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses `dribbling_client_is_dropped_at_the_total_deadline`. The
    /// dribbler paces itself with an interval (pacing the client is the
    /// behaviour under test, not a wait for a condition); the assertion is
    /// that the server ends the connection near `io_timeout`.
    #[tokio::test]
    async fn dribbling_client_is_dropped_at_the_deadline() {
        const IO_TIMEOUT_MS: u64 = 300;
        let (srv, _tx) = start(cfg(8, IO_TIMEOUT_MS)).await;
        let mut s = TcpStream::connect(srv.addr).await.unwrap();
        let start = tokio::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(40));
        let mut buf = [0u8; 256];
        let closed_after = loop {
            tokio::select! {
                _ = tick.tick() => { if s.write_all(b"G").await.is_err() { break start.elapsed(); } }
                r = s.read(&mut buf) => match r {
                    Ok(0) | Err(_) => break start.elapsed(),
                    Ok(n) => {
                        // hyper may answer a timed-out request head with a
                        // status line before closing; that is a close, not a metrics body.
                        assert!(!String::from_utf8_lossy(&buf[..n]).contains("media_doctor_packets_total"),
                            "an incomplete request head must not get the metrics body");
                    }
                },
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "server never dropped the dribbler"
            );
        };
        assert!(
            closed_after >= std::time::Duration::from_millis(IO_TIMEOUT_MS / 2),
            "dropped far too early: {closed_after:?}"
        );
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// New: shutdown ends `serve`, and the port is free again at once
    /// (the accept pump must not leak its bound socket — spec defect class 3).
    #[tokio::test]
    async fn shutdown_releases_the_port() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let addr = srv.addr;
        let _idle = TcpStream::connect(addr).await.unwrap();
        srv.token.cancel();
        srv.task.await.unwrap();
        std::net::TcpListener::bind(addr).expect("port must be released after shutdown");
    }

    /// Shutdown is prompt even with an idle connection open and a long
    /// deadline (graceful shutdown closes idle connections at once).
    #[tokio::test]
    async fn shutdown_with_an_idle_connection_returns_promptly() {
        let (srv, _tx) = start(cfg(8, 60_000)).await;
        let _idle = TcpStream::connect(srv.addr).await.unwrap();
        // Let the accept loop admit it: a served request proves ordering.
        assert!(scrape(srv.addr).await.starts_with("HTTP/1.1 200 OK"));
        srv.token.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(5), srv.task)
            .await
            .expect("serve must return well inside the 60 s deadline")
            .unwrap();
    }

    /// An in-flight response completes on shutdown: the body is far larger
    /// than the socket buffers, so the server is still writing when the token
    /// is cancelled (the client has read only the first bytes). The client
    /// must still receive the whole body. Fails if connection futures are
    /// dropped abruptly on cancel instead of shut down gracefully.
    #[tokio::test]
    async fn in_flight_response_completes_on_shutdown() {
        const BODY_LEN: usize = 32 * 1024 * 1024;
        let (srv, tx) = start(cfg(8, 60_000)).await;
        let big: String = "x".repeat(BODY_LEN);
        tx.send_replace(Arc::<str>::from(big));
        let mut s = TcpStream::connect(srv.addr).await.unwrap();
        s.write_all(REQ).await.unwrap();
        // Read just the start of the response: the server is now blocked
        // writing the rest into a full socket buffer.
        let mut got = vec![0u8; 4096];
        s.read_exact(&mut got).await.unwrap();
        assert!(got.starts_with(b"HTTP/1.1 200 OK"));
        srv.token.cancel();
        // Drain the rest: the whole body must arrive.
        let mut rest = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(30), s.read_to_end(&mut rest))
            .await
            .expect("drain timed out")
            .unwrap();
        got.extend_from_slice(&rest);
        let split = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(
            got.len() - split,
            BODY_LEN,
            "the response was truncated by shutdown"
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), srv.task)
            .await
            .expect("serve must return")
            .unwrap();
    }

    /// New: a body published after start is served by the next scrape.
    #[tokio::test]
    async fn scrape_serves_the_latest_published_body() {
        let (srv, tx) = start(cfg(8, 3_000)).await;
        tx.send_replace(Arc::<str>::from("# TYPE x gauge\nx 42\n"));
        assert!(scrape(srv.addr).await.contains("\nx 42\n"));
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Re-expresses the HTTP golden (Task 1) semantically: same status,
    /// same content type, `Content-Length` equals the body, `Connection:
    /// close`; hyper additionally sends `date` and lower-cases names.
    #[tokio::test]
    async fn response_head_matches_the_main_golden_after_normalisation() {
        let (srv, _tx) = start(cfg(8, 3_000)).await;
        let text = scrape(srv.addr).await;
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let norm = |h: &str| {
            let mut lines: Vec<String> = h
                .lines()
                .skip(1)
                .map(|l| l.to_ascii_lowercase())
                .filter(|l| !l.starts_with("date:") && !l.starts_with("content-length:"))
                .collect();
            lines.sort();
            lines
        };
        let golden = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/golden/watch/http-response-head.txt"
        ))
        .unwrap();
        assert_eq!(head.lines().next(), golden.lines().next(), "status line");
        assert_eq!(norm(head), norm(golden.trim_end()), "headers");
        assert!(
            head.to_ascii_lowercase()
                .contains(&format!("content-length: {}", body.len()))
        );
        srv.token.cancel();
        srv.task.await.unwrap();
    }

    /// Publisher cadence is driven by the caller's clock (deterministic).
    #[test]
    fn publisher_renders_at_most_once_per_interval_and_flushes_on_demand() {
        let state = WatchState::new();
        let (mut p, rx) = channel(&state, std::time::Duration::from_millis(250));
        let t = std::time::Duration::from_millis;
        assert!(p.maybe_publish(&state, t(0)), "first call publishes");
        assert!(
            !p.maybe_publish(&state, t(100)),
            "inside the interval: skipped"
        );
        assert!(!p.maybe_publish(&state, t(249)));
        assert!(
            p.maybe_publish(&state, t(250)),
            "at the interval: published"
        );
        p.publish_now(&state, t(251));
        assert!(
            rx.borrow().contains("media_doctor_packets_total 0"),
            "fresh state exposes every family"
        );
    }
}
