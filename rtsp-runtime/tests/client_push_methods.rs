//! `AsyncRtspClient::announce`/`record`/`send_interleaved` (W2b-1 Task 6) — the
//! RTSP push (ANNOUNCE -> SETUP -> RECORD with interleaved delivery) needs them.
#![cfg(feature = "tokio")]

use std::time::Duration;

use rtsp_runtime::client::ClientEvent;
use rtsp_runtime::server::ServerEvent;
use rtsp_runtime::{
    AsyncRtspClient, AsyncRtspServer, ClientSession, RtspTimeouts, StatusCode, Transport,
    TransportSpec,
};
use tokio::io::duplex;
use tokio::net::TcpListener;

const URI: &str = "rtsp://127.0.0.1/live";

fn tcp_interleaved() -> Transport {
    Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))
}

const SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n";

#[tokio::test]
async fn announce_then_record_round_trips_over_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        // ANNOUNCE, SETUP, RECORD = 3 requests.
        let mut methods = Vec::new();
        for _ in 0..3 {
            let events = srv.next_request().await.unwrap().expect("request");
            for e in events {
                if let ServerEvent::RequestAccepted { method, .. } = e {
                    methods.push(method);
                }
            }
        }
        methods
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    let ev = client.announce(URI, SDP).await.unwrap();
    assert!(
        matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok),
        "{ev:?}"
    );
    let ev = client.setup(URI, &tcp_interleaved()).await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { .. }), "{ev:?}");
    let ev = client.record(URI).await.unwrap();
    assert!(
        matches!(ev, ClientEvent::Response { status, .. } if status == StatusCode::Ok),
        "{ev:?}"
    );

    let methods = server.await.unwrap();
    assert_eq!(methods.len(), 3, "server saw {methods:?}");
}

#[tokio::test]
async fn a_client_interleaved_send_arrives_as_a_frame_at_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        let mut srv = AsyncRtspServer::accept(sock);
        loop {
            let events = srv.next_request().await.unwrap()?;
            for e in events {
                if let ServerEvent::MediaData { channel, data } = e {
                    return Some((channel, data));
                }
            }
        }
    });

    let mut client = AsyncRtspClient::connect(addr).await.unwrap();
    client.send_interleaved(0, b"media-payload").await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server must receive the interleaved frame")
        .unwrap()
        .expect("a MediaData event");
    assert_eq!(got.0, 0, "channel");
    assert_eq!(got.1, b"media-payload".to_vec());
}

#[tokio::test(start_paused = true)]
async fn a_stalled_interleaved_send_times_out_at_the_write_bound() {
    // An 8-byte duplex pipe whose peer is kept alive but never drained: the
    // write blocks, and the `write` bound must fail it (paused virtual time).
    let (client_io, _server_io) = duplex(8);
    let timeouts = RtspTimeouts::default().with_write(Duration::from_secs(1));
    let mut client =
        AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), timeouts);
    // Wrapped in an outer virtual-time bound so a MISSING write bound fails
    // with a clear message instead of hanging the suite.
    let err = tokio::time::timeout(Duration::from_secs(5), client.send_interleaved(0, &[0u8; 64]))
        .await
        .expect("the stalled send must fail at the write bound, not hang")
        .expect_err("the stalled send must fail at the write bound");
    assert!(
        matches!(err, rtsp_runtime::Error::Timeout { what: "write" }),
        "got {err:?}"
    );
}
