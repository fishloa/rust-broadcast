#![cfg(all(feature = "whep", feature = "test-hooks"))]
//! SP1.4 / defect 3: every detached spawn a route makes is owned by a tracked
//! task, and cancelling the route's token drains them all and releases the
//! route's listen port.

use std::time::Duration;

use tokio::net::TcpListener;

#[tokio::test]
async fn cancelling_a_whep_route_drains_its_session_tracker_and_releases_the_port() {
    // `serve_whep_run_for_test` runs the REAL `run_whep` and hands back the
    // route's own session `TaskTracker`. Cancelling the token must make
    // `run_whep` return — i.e. every tracked session has drained — and release
    // the port.
    let (addr, sessions, handle, cancel) = multimux::output::whep::serve_whep_run_for_test().await;

    // Wait until the signalling server is actually accepting.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the WHEP signalling server never bound {addr}"
        );
        tokio::task::yield_now().await;
    }

    // Admit a real viewer session (a POST /whep with a valid offer), so the
    // tracker is non-empty before cancel — otherwise the drain below would be
    // vacuous.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/whep"))
        .header(reqwest::header::CONTENT_TYPE, "application/sdp")
        .body(multimux::output::whep::WHEP_TEST_OFFER)
        .send()
        .await
        .expect("POST /whep must reach the WHEP signalling server");
    assert!(
        resp.status().is_success(),
        "the WHEP offer must be admitted: {}",
        resp.status()
    );

    // The session task is now tracked (defect 3: `sessions.spawn`, not a bare
    // `tokio::spawn` whose handle nothing observes).
    assert!(
        !sessions.is_empty(),
        "the admitted session must be owned by the route's TaskTracker"
    );

    cancel.cancel();

    // `run_whep` must return once its tracked tasks drain (it `close()`s and
    // `wait()`s the session tracker before returning).
    let drained = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(
        drained.is_ok(),
        "run_whep must return after cancel drains its tracked tasks"
    );
    assert!(
        sessions.is_empty(),
        "the tracker must be drained after cancel"
    );

    // And the port is free again.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpListener::bind(addr).await {
            Ok(_) => break,
            Err(_) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Err(e) => panic!("the route's port {addr} stayed bound after cancel: {e}"),
        }
    }
}
