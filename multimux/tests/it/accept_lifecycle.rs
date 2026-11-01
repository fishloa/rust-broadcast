#![cfg(all(feature = "whip", feature = "whep", feature = "test-hooks"))]
//! Defects 2 and 3: the RTMP/WHIP/WHEP accept pumps are tracked tasks, a
//! blocked accept permit is cancelled on shutdown, and accepts are admitted
//! under steady reads (no 20 ms sleep-poll).

use std::time::Duration;

use tokio::net::TcpListener;

async fn wait_for_rebind(addr: std::net::SocketAddr, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpListener::bind(addr).await {
            Ok(_) => return,
            Err(_) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Err(e) => panic!("{what} port {addr} stayed bound after drop/cancel: {e}"),
        }
    }
}

#[tokio::test]
async fn dropping_a_whip_listener_releases_its_port() {
    for _ in 0..20 {
        let (addr, shared, token) = multimux::source::whip::serve_for_test().await;
        drop(shared);
        token.cancel();
        wait_for_rebind(addr, "whip").await;
    }
}

#[tokio::test]
async fn cancelling_a_saturated_whep_accept_returns_and_releases_the_port() {
    let (addr, token) = multimux::output::whep::serve_whep_for_test_saturated_accept().await;
    token.cancel();
    wait_for_rebind(addr, "whep").await;
}

#[tokio::test]
async fn an_rtmp_accept_is_admitted_while_a_read_is_continuously_ready() {
    // Defect 2 (RTMP): the pre-fix accept arm is a 20 ms sleep-poll that a
    // steady read load starves. Publisher A connects and we keep its
    // connection continuously readable (byte-at-a-time); publisher B must
    // still be admitted well inside the 20 ms × N window the old code needs.
    let (server_addr, route, token) = multimux::source::rtmp::serve_for_test_with_read_load().await;

    // Publisher A: hold a connection whose read side is always ready.
    use tokio::io::AsyncWriteExt as _;
    let mut a = tokio::net::TcpStream::connect(server_addr).await.unwrap();
    a.write_all(&[0x03]).await.unwrap(); // RTMP C0, never the rest of the handshake
    // A steady trickle keeps the source's read future always-ready.
    let drivel = tokio::spawn(async move {
        let mut i = 0u8;
        loop {
            let _ = a.write_all(&[i]).await;
            i = i.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // Publisher B must be admitted (its C0/C1 read by the accept pump)
    // within a bound far tighter than the sleep-starved old loop. The result
    // is captured FIRST so `drivel`/the server can be torn down before we
    // assert, otherwise a failure leaks the forever-looping tasks and hangs
    // the test binary instead of reporting the assertion.
    let mut b = tokio::net::TcpStream::connect(server_addr).await.unwrap();
    b.write_all(&[0x03]).await.unwrap();
    let admitted = route.wait_for_sessions(2, Duration::from_millis(500)).await;
    drivel.abort();
    token.cancel();
    std::mem::drop(b);
    let admitted = admitted.expect("the second publisher must be admitted under a steady read");
    assert_eq!(admitted, 2, "two connections must be accepted");
}
