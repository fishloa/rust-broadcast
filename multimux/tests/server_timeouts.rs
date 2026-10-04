//! SP2.1: every multimux HTTP listener is served through one hyper-util
//! server with a timer and a header-read timeout, a connection cap enforced
//! at `accept`, and a graceful shutdown that drains in-flight responses up to
//! a drain deadline (never truncating a long-lived body mid-stream).
//!
//! DEVIATION from the plan: these use **real** time, not
//! `#[tokio::test(start_paused = true)]`. The header-read deadline is a
//! hyper-internal `TokioTimer` sleep armed only once the accepted connection
//! first reaches `poll_read_head`; under a paused clock the real-socket
//! `accept`/read readiness the server depends on is never delivered by a
//! `yield_now` loop, so the timer is armed after the test has already
//! advanced past it. Real time with short timeouts and bounded condition
//! waits is deterministic (the bound is a condition wait, not a fixed sleep).

use std::sync::Arc;
use std::time::Duration;

use multimux::origin::{AppState, HttpLimits};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

fn origin_app() -> axum::Router {
    let state = Arc::new(AppState::new(Default::default()).with_limits(HttpLimits::default()));
    multimux::origin::router(state)
}

/// A router whose `/slow` route streams one chunk, waits, then another, and a
/// `/peer` route replies with the transport peer address (what `ConnectInfo`
/// carries). Both bodies are long enough to outlive any old total deadline.
fn streaming_app() -> axum::Router {
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::StatusCode;
    use axum::routing::get;
    fn two_chunk_stream() -> axum::body::Body {
        axum::body::Body::from_stream(futures_util::stream::unfold(0u8, move |i| async move {
            match i {
                0 => {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    Some((
                        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first")),
                        1,
                    ))
                }
                1 => Some((Ok(axum::body::Bytes::from_static(b"-second")), 2)),
                _ => None,
            }
        }))
    }
    axum::Router::new()
        .route("/slow", get(|| async { two_chunk_stream() }))
        .route(
            "/peer",
            get(
                |ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>| async move {
                    (StatusCode::OK, peer.to_string())
                },
            ),
        )
}

#[tokio::test]
async fn a_slow_header_client_is_dropped_at_the_header_read_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let app = origin_app();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_millis(200),
        Duration::from_secs(5),
    ));

    // A slow-loris client: half a request head, then silence.
    let mut slow = TcpStream::connect(addr).await.unwrap();
    slow.write_all(b"POST / HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();

    // The 200 ms header timeout must close it, not hold it open.
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut buf)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Ok(Err(_))),
        "the slow-header connection must be closed, not held open: {read:?}"
    );

    token.cancel();
    let _ = serve.await;
}

/// (N1) Finished connection tasks are reaped continuously, not retained for
/// the life of the listener: after many sequential short connections the
/// live task count returns to zero (a leak would leave it at one entry per
/// connection ever accepted).
#[tokio::test]
async fn finished_connection_tasks_are_reaped_not_accumulated() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_task_gauge(
        listener,
        streaming_app(),
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        Arc::clone(&live),
    ));

    // Many sequential short connections: each completes immediately, so the
    // reaper must drain the JoinSet as it goes. Without the reaping fix, the
    // gauge stays pinned near N (one spawned entry per connection, never
    // decremented until the listener exits).
    for _ in 0..64 {
        let mut c = TcpStream::connect(addr).await.unwrap();
        c.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut body = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), c.read_to_end(&mut body))
            .await
            .expect("every connection must complete")
            .unwrap();
    }

    // Wait until the gauge settles to a bounded value. The newest finished
    // connection is reaped at the top of the next accept iteration, so one
    // entry may linger until the next connection arrives; the point is that
    // the set does NOT accumulate one entry per connection (which would pin
    // `live` at 64). Bound this with a condition wait, not a sleep.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let live = live.load(std::sync::atomic::Ordering::Relaxed);
        if live <= 1 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "finished connection tasks must be reaped, live = {live} (leaked entries \
                 across 64 sequential connections)"
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    token.cancel();
    let _ = serve.await;
}

#[tokio::test]
async fn the_listener_still_serves_a_good_request_after_a_slow_client() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let app = origin_app();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_millis(200),
        Duration::from_secs(5),
    ));

    // A client that stops mid-head.
    let mut slow = TcpStream::connect(addr).await.unwrap();
    slow.write_all(b"POST / HTTP/1.1\r\n").await.unwrap();
    let mut buf = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut buf)).await;

    // The listener still answers a normal request.
    let mut good = TcpStream::connect(addr).await.unwrap();
    good.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut good, &mut body)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&body).starts_with("HTTP/1.1 200"),
        "healthz must still answer: {}",
        String::from_utf8_lossy(&body)
    );

    token.cancel();
    let _ = serve.await;
}

/// (b) A response longer than the old 30 s "total deadline" is NOT cut: there
/// is no total per-connection deadline at all. The `/slow` body spans >150 ms
/// of wall time across two chunks, and both must arrive.
#[tokio::test]
async fn a_response_longer_than_any_total_deadline_is_not_cut() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    // `drain` here is a tiny 10 ms: if it were wrongly applied as a
    // per-connection total deadline, the second chunk would be cut.
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        streaming_app(),
        token.clone(),
        Duration::from_secs(5),
        Duration::from_millis(10),
    ));

    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), c.read_to_end(&mut body))
        .await
        .expect("the slow body must finish, not be cut")
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("first") && body.contains("second"),
        "both chunks must arrive un-truncated: {body}"
    );

    token.cancel();
    let _ = serve.await;
}

/// (b2) The old 30 s "total per-connection deadline" is GONE: a response body
/// spanning more than 30 s (measured in paused clock) still completes. This
/// uses a paused clock so the >30 s span is deterministic, not a 30 s wall
/// wait — the 150 ms real-time test above cannot prove the 30 s bound does
/// not exist.
#[tokio::test(start_paused = true)]
async fn a_response_longer_than_thirty_seconds_is_not_cut() {
    use axum::routing::get;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();

    // A body that streams one chunk, waits 31 s, then another. Under a paused
    // clock the 31 s span is advanced deterministically.
    let app = axum::Router::new().route(
        "/slow",
        get(|| async {
            axum::body::Body::from_stream(futures_util::stream::unfold(0u8, |i| async move {
                match i {
                    0 => {
                        tokio::time::sleep(Duration::from_secs(31)).await;
                        Some((
                            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first")),
                            1,
                        ))
                    }
                    1 => Some((Ok(axum::body::Bytes::from_static(b"-second")), 2)),
                    _ => None,
                }
            }))
        }),
    );

    // `header_read`/`drain` are irrelevant here; `drain` is only the shutdown
    // drain, never a per-connection total deadline.
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    // Advance the paused clock well past 30 s: the body must still complete
    // (no total deadline cuts it). Advance in small steps, yielding so the
    // server's paused sleep fires and the chunks are written to the socket.
    let read = tokio::spawn(async move {
        let mut buf = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut c, &mut buf)
            .await
            .unwrap();
        buf
    });
    for _ in 0..70 {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        if read.is_finished() {
            break;
        }
    }
    assert!(
        read.is_finished(),
        "a >30 s response must complete, not be cut by a total deadline"
    );
    let body = read.await.unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("first") && body.contains("second"),
        "both chunks must arrive un-truncated across >30 s: {body}"
    );

    token.cancel();
    let _ = serve.await;
}

/// (a) An in-flight response with a slow body COMPLETES across shutdown: the
/// server stops accepting, then drains the open connection to its natural end
/// rather than cutting it when the cancel token fires.
#[tokio::test]
async fn an_in_flight_response_completes_on_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();

    // A handler that streams two chunks: the first immediately, the second
    // only after the test triggers shutdown (proving shutdown never cuts an
    // in-flight body).
    use axum::routing::get;
    let gate = Arc::new(tokio::sync::Notify::new());
    let app = axum::Router::new().route(
        "/slow",
        get({
            let gate = Arc::clone(&gate);
            move || {
                let gate = Arc::clone(&gate);
                async move {
                    axum::body::Body::from_stream(futures_util::stream::unfold(0u8, move |i| {
                        let gate = Arc::clone(&gate);
                        async move {
                            match i {
                                0 => Some((
                                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                                        b"first",
                                    )),
                                    1,
                                )),
                                1 => {
                                    gate.notified().await;
                                    Some((Ok(axum::body::Bytes::from_static(b"-second")), 2))
                                }
                                _ => None,
                            }
                        }
                    }))
                }
            }
        }),
    );

    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let mut c = TcpStream::connect(addr).await.unwrap();
    c.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    // Read the first chunk so the request is firmly in flight.
    let mut first = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(2), c.read(&mut first))
        .await
        .expect("reads the first chunk")
        .unwrap();
    assert!(n > 0 && String::from_utf8_lossy(&first[..n]).contains("first"));

    // Trigger graceful shutdown while the response is still streaming.
    token.cancel();

    // Now let the second chunk through (the server is draining); the body
    // must still fully complete even though shutdown fired mid-stream.
    // `notify_one` (not `notify_waiters`) stores a permit so the wake is NOT
    // lost if the handler has not yet reached its `notified().await` — the
    // second chunk is polled only once shutdown begins, so the waiter can
    // arrive after this call.
    gate.notify_one();
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), c.read_to_end(&mut rest))
        .await
        .expect("the in-flight body must drain to completion across shutdown")
        .unwrap();
    let mut whole = first[..n].to_vec();
    whole.extend_from_slice(&rest);
    let whole = String::from_utf8_lossy(&whole).into_owned();
    assert!(
        whole.contains("first") && whole.contains("second"),
        "the streamed body must complete across shutdown: {whole}"
    );

    let _ = serve.await;
}

/// (c) `ConnectInfo` reaches the handler: the injected peer address is the
/// transport remote, not a placeholder.
#[tokio::test]
async fn connect_info_reaches_the_handler() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        streaming_app(),
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
    ));

    let mut c = TcpStream::connect(addr).await.unwrap();
    let mine = c.local_addr().unwrap();
    c.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    c.read_to_end(&mut body).await.unwrap();
    let body = String::from_utf8_lossy(&body);
    let mine_str = mine.to_string();
    assert!(
        body.contains(&mine_str),
        "the handler must see the real peer {mine_str}, not a placeholder: {body}"
    );

    token.cancel();
    let _ = serve.await;
}

/// (d) The connection cap closes over-cap sockets at `accept` (they get an
/// immediate EOF, and none accumulate).
#[tokio::test]
async fn the_connection_cap_closes_over_cap_sockets_at_accept() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let app = streaming_app();
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_limits(
        listener,
        app,
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
        1, // cap = 1 live connection
    ));

    // First connection: an idle keep-alive client holds the only permit by
    // sending a request head and keeping the socket open (no data consumed).
    let mut held = TcpStream::connect(addr).await.unwrap();
    held.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();

    // Second connection: past the cap of 1. It must be closed at accept (EOF),
    // not parked. Give the server a moment to accept + close it.
    let mut over = TcpStream::connect(addr).await.unwrap();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), over.read(&mut buf)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Ok(Err(_))),
        "the over-cap socket must be closed at accept, not held: {read:?}"
    );

    drop(held);
    token.cancel();
    let _ = serve.await;
}

/// An `accept()` error (EMFILE) does not end the listener: it backs off and
/// the next real connection is still served.
#[tokio::test]
async fn an_accept_error_does_not_kill_the_listener() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let app = streaming_app();

    // Fail the first `n` accepts with EMFILE, then delegate to the real
    // listener's `accept`.
    let fail_first = Arc::new(std::sync::atomic::AtomicUsize::new(3));
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_accept_source(
        listener,
        app,
        token.clone(),
        Duration::from_secs(5),
        Duration::from_secs(5),
        {
            let fail_first = Arc::clone(&fail_first);
            move |l: Arc<tokio::net::TcpListener>| {
                let fail_first = Arc::clone(&fail_first);
                Box::pin(async move {
                    if fail_first
                        .fetch_update(
                            std::sync::atomic::Ordering::SeqCst,
                            std::sync::atomic::Ordering::SeqCst,
                            |n| if n > 0 { Some(n - 1) } else { None },
                        )
                        .is_ok()
                    {
                        Err(std::io::Error::from_raw_os_error(24 /* EMFILE */))
                    } else {
                        l.accept().await
                    }
                })
            }
        },
    ));

    // The listener survives the transient errors and still serves a request.
    let mut good = TcpStream::connect(addr).await.unwrap();
    good.write_all(b"GET /peer HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), good.read_to_end(&mut body))
        .await
        .expect("the listener must still serve after accept errors")
        .unwrap();
    assert!(
        String::from_utf8_lossy(&body).starts_with("HTTP/1.1 200"),
        "a request after transient accept errors must succeed: {}",
        String::from_utf8_lossy(&body)
    );

    token.cancel();
    let _ = serve.await;
}
