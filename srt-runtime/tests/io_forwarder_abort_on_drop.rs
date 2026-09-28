//! Item 3 — dropping a [`SrtSocket::connect_from`]ed caller must abort its
//! dedicated-socket forwarder task, not merely drop the [`Driver`]'s own
//! `Arc<UdpSocket>` reference.
//!
//! Pre-fix, `spawn_dedicated_socket_forwarder`'s task held its own clone of
//! that `Arc` and looped on `recv_from` forever; dropping `SrtSocket` only
//! dropped the `Driver`'s reference, so the forwarder — with no datagram ever
//! arriving to make its next `tx.send` fail — stayed parked on the socket
//! indefinitely, keeping the local port bound. A later attempt to bind that
//! same local address then failed with `AddrInUse`.

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};

async fn free_local_addr() -> std::net::SocketAddr {
    tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
}

#[tokio::test]
async fn caller_drop_releases_its_local_port_for_reuse() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .expect("bind listener");
    let listener_addr = listener.local_addr().expect("listener local addr");
    let local = free_local_addr().await;

    let accept_jh = tokio::spawn(async move {
        let s = listener.accept().await.expect("accept");
        (listener, s)
    });
    let caller = SrtSocket::connect_from(local, listener_addr, HandshakeConfig::default())
        .await
        .expect("connect_from");
    let (_listener, accepted) = accept_jh.await.expect("join accept");

    drop(caller);
    drop(accepted);

    // Task abort is scheduled, not synchronous with `drop` — poll for the
    // port actually freeing up rather than sleeping a fixed guess.
    let mut freed = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if tokio::net::UdpSocket::bind(local).await.is_ok() {
            freed = true;
            break;
        }
    }
    assert!(
        freed,
        "local port {local} still held after the caller `SrtSocket` was dropped \
         — the dedicated-socket forwarder task leaked (pre-fix: it holds its own \
         `Arc<UdpSocket>` clone and is never explicitly aborted)"
    );
}
