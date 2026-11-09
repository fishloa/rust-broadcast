#![cfg(feature = "tokio")]
//! The push drain must not turn a decode error into a clean EOF.

use rtsp_runtime::{AsyncRtspClient, ClientSession, Error};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn a_decode_error_swallowed_by_the_push_drain_still_surfaces_as_a_parse_error() {
    let (client_io, mut server_io) = tokio::io::duplex(64 * 1024);
    let mut client = AsyncRtspClient::with_stream(client_io, ClientSession::new());
    server_io.write_all(b"GARBAGE\r\n\r\n").await.unwrap();
    server_io.flush().await.unwrap();

    let _ = client.send_interleaved(0, b"rtp").await;
    let r = tokio::time::timeout(std::time::Duration::from_secs(2), client.recv_interleaved())
        .await
        .expect("recv_interleaved hung");
    assert!(
        matches!(r, Err(Error::MessageParse { .. })),
        "expected the parse error to surface, got {:?}",
        r.map(|o| o.map(|_| ()))
    );
}
