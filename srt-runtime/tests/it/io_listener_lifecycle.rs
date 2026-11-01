#![cfg(feature = "tokio")]

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};

const WAIT: Duration = Duration::from_secs(10);

/// Waits (on the real condition, bounded) until `addr` can be bound again.
async fn port_is_released(addr: SocketAddr) -> bool {
    tokio::time::timeout(WAIT, async {
        loop {
            if UdpSocket::bind(addr).is_ok() {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

/// Defect 3: the routing pump held an `Arc<UdpSocket>` in `recv_from` and only noticed a dropped
/// listener when the NEXT datagram arrived, so an idle listener kept its port bound forever.
#[tokio::test]
async fn dropping_an_idle_listener_releases_its_udp_port() {
    let listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    assert!(
        port_is_released(addr).await,
        "the UDP port stayed bound after the listener was dropped"
    );
}

/// Review-focus 1: dropping the listener HANDLE must not kill connections it already accepted.
#[tokio::test]
async fn an_accepted_connection_outlives_the_listener_handle_and_then_releases_the_port() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (s, listener)) });
    let mut caller = SrtSocket::connect(addr, HandshakeConfig::default())
        .await
        .unwrap();
    let (mut accepted, listener) = tokio::time::timeout(WAIT, accept)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(listener); // handle gone, connection live
    caller.send(b"still routed").await.unwrap();
    let got = tokio::time::timeout(WAIT, accepted.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        &got[..],
        b"still routed",
        "routing pump must keep serving accepted connections"
    );
    assert!(
        UdpSocket::bind(addr).is_err(),
        "port is still in use while a connection is alive"
    );
    drop(accepted);
    drop(caller);
    assert!(
        port_is_released(addr).await,
        "port must be released once the last connection is gone"
    );
}

/// Handshakes used to advance only while `accept()` was being polled.
#[tokio::test]
async fn a_caller_connects_even_though_accept_is_not_being_polled() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let caller = tokio::time::timeout(
        Duration::from_secs(3),
        SrtSocket::connect(addr, HandshakeConfig::default()),
    )
    .await
    .expect("handshake must complete without anyone calling accept()")
    .unwrap();
    let _accepted = tokio::time::timeout(WAIT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    drop(caller);
}

/// After the handle is dropped, a NEW handshake (which can no longer be handed
/// to `accept`) must not stop the routing of connections already accepted.
#[tokio::test]
async fn a_handshake_after_the_handle_is_gone_does_not_break_an_accepted_connection() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (s, listener)) });
    let mut caller = SrtSocket::connect(addr, HandshakeConfig::default())
        .await
        .unwrap();
    let (mut accepted, listener) = tokio::time::timeout(WAIT, accept)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(listener);
    // A second caller completes its handshake against the handle-less listener.
    let _late = tokio::time::timeout(
        Duration::from_secs(3),
        SrtSocket::connect(addr, HandshakeConfig::default()),
    )
    .await;
    // More unrouted traffic (destination socket id 0) after the core stopped
    // serving handshakes: the routing pump must keep running regardless.
    let raw = UdpSocket::bind("127.0.0.1:0").unwrap();
    for _ in 0..3 {
        raw.send_to(&[0u8; 32], addr).unwrap();
        tokio::task::yield_now().await;
    }
    caller.send(b"still routed").await.unwrap();
    let got = tokio::time::timeout(WAIT, accepted.recv())
        .await
        .expect("the accepted connection stopped receiving after a late handshake")
        .unwrap()
        .unwrap();
    assert_eq!(&got[..], b"still routed");
}
