//! Self-vs-self test of the tokio adapter's Rendezvous mode
//! ([`SrtSocket::rendezvous`]): two sockets, each bound to a known loopback
//! port and targeting the other, complete the handshake and exchange data in
//! both directions. Runs everywhere (no external tool); the independent
//! oracle for the same flow is `libsrt_rendezvous.rs`.
#![cfg(feature = "tokio")]

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use srt_runtime::Error;
use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{IoConfig, SrtSocket};

fn free_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn two_rendezvous_sockets_connect_and_exchange_data() {
    for _ in 0..4 {
        let (pa, pb) = (free_port(), free_port());
        let a_addr: SocketAddr = format!("127.0.0.1:{pa}").parse().unwrap();
        let b_addr: SocketAddr = format!("127.0.0.1:{pb}").parse().unwrap();
        let a = SrtSocket::rendezvous(a_addr, b_addr, HandshakeConfig::default());
        let b = SrtSocket::rendezvous(b_addr, a_addr, HandshakeConfig::default());
        let (a, b) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(a, b) })
            .await
            .expect("rendezvous never completed");
        let (mut a, mut b) = (a.expect("a"), b.expect("b"));

        a.send(b"from a").await.unwrap();
        b.send(b"from b").await.unwrap();
        let got_b = tokio::time::timeout(Duration::from_secs(10), b.recv())
            .await
            .expect("b recv timed out")
            .unwrap()
            .expect("b got data");
        let got_a = tokio::time::timeout(Duration::from_secs(10), a.recv())
            .await
            .expect("a recv timed out")
            .unwrap()
            .expect("a got data");
        assert_eq!(&got_b[..], b"from a");
        assert_eq!(&got_a[..], b"from b");
    }
}

/// A rendezvous whose peer never shows up fails with the typed timeout, not a
/// hang.
#[tokio::test]
async fn rendezvous_without_a_peer_times_out() {
    let (pa, pb) = (free_port(), free_port());
    let err = SrtSocket::rendezvous_with(
        format!("127.0.0.1:{pa}").parse().unwrap(),
        format!("127.0.0.1:{pb}").parse::<SocketAddr>().unwrap(),
        HandshakeConfig::default(),
        IoConfig::default().with_handshake(Duration::from_millis(600)),
    )
    .await
    .map(|_| ())
    .expect_err("no peer must fail");
    assert!(
        matches!(err, Error::HandshakeTimedOut { .. } | Error::Io { .. }),
        "{err:?}"
    );
}
