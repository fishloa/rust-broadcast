#![cfg(feature = "tokio")]

use std::time::Duration;

use srt_runtime::HandshakeConfig;
use srt_runtime::io::{IoConfig, SrtListener, SrtSocket};

/// A non-default `max_datagram` is honoured end to end, and `recv` hands out `Bytes`.
#[tokio::test]
async fn a_custom_max_datagram_connection_round_trips_a_payload() {
    let io = IoConfig::default().with_max_datagram(9000); // jumbo-frame path
    let mut listener = SrtListener::bind_with("127.0.0.1:0", HandshakeConfig::default(), io)
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (listener, s)) });
    let mut caller = SrtSocket::connect_with(addr, HandshakeConfig::default(), io)
        .await
        .unwrap();
    let (_listener, mut accepted) = tokio::time::timeout(Duration::from_secs(10), accept)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    caller
        .send(b"hello over a custom datagram size")
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(10), accepted.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let got: bytes::Bytes = got;
    assert_eq!(&got[..], b"hello over a custom datagram size");
    assert_eq!(accepted.stats().rx_oversize, 0);
}
