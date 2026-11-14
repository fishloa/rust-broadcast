//! Self-vs-self test of the tokio adapter's Rendezvous mode
//! ([`SrtSocket::rendezvous`]): two sockets, each bound to a known loopback
//! port and targeting the other, complete the handshake and exchange data in
//! both directions. Runs everywhere (no external tool); the independent
//! oracle for the same flow is `libsrt_rendezvous.rs`.
#![cfg(feature = "tokio")]

use std::time::Duration;

use srt_runtime::Error;
use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{IoConfig, SRT_TIMEOUTS, SrtSocket};
use tokio::net::UdpSocket;

#[tokio::test]
async fn two_rendezvous_sockets_connect_and_exchange_data() {
    for _ in 0..4 {
        let sock_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let a_addr = sock_a.local_addr().unwrap();
        let sock_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let b_addr = sock_b.local_addr().unwrap();
        let a = SrtSocket::rendezvous_from_socket(
            sock_a,
            b_addr,
            srt_runtime::io::SrtSocketConfig::new(HandshakeConfig::default())
                .with_io(IoConfig::default()),
        );
        let b = SrtSocket::rendezvous_from_socket(
            sock_b,
            a_addr,
            srt_runtime::io::SrtSocketConfig::new(HandshakeConfig::default())
                .with_io(IoConfig::default()),
        );
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
    let sock_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    // The peer address is a port we release on purpose: nothing answers there.
    let sock_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let b_addr = sock_b.local_addr().unwrap();
    drop(sock_b);
    let err = SrtSocket::rendezvous_from_socket(
        sock_a,
        b_addr,
        srt_runtime::io::SrtSocketConfig::new(HandshakeConfig::default()).with_io(
            IoConfig::default()
                .with_timeouts(SRT_TIMEOUTS.with_handshake(Duration::from_millis(600))),
        ),
    )
    .await
    .map(|_| ())
    .expect_err("no peer must fail");
    assert!(
        matches!(err, Error::HandshakeTimedOut { .. } | Error::Io { .. }),
        "{err:?}"
    );
}
