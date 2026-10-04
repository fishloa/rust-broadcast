#![cfg(all(feature = "whip", feature = "test-hooks"))]
//! SP2.1: the WHIP signalling endpoint is served by an axum `Router` on
//! hyper-util — chunked bodies are accepted, the preflight carries CORS, and
//! an oversized body is rejected 413 before it is read.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";

async fn raw(addr: std::net::SocketAddr, req: &str) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    // The request line/headers are written with bare `\n`; the SDP body
    // already carries its own CRLF, so normalise only a lone LF to CRLF.
    let req = req.replace("\r\n", "\n").replace('\n', "\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

async fn post_offer(addr: std::net::SocketAddr) -> String {
    let body = OFFER.to_string();
    raw(
        addr,
        &format!(
            "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\n\
             Content-Length: {}\nConnection: close\n\n{body}",
            body.len()
        ),
    )
    .await
}

#[tokio::test]
async fn whip_post_is_answered_201_with_a_location_and_typed_content_type() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = post_offer(addr).await;
    let lower = resp.to_ascii_lowercase();
    assert!(resp.starts_with("HTTP/1.1 201"), "{resp}");
    assert!(lower.contains("content-type: application/sdp"), "{resp}");
    assert!(
        lower.contains("location:"),
        "RFC 9725 §4.1 mandates Location: {resp}"
    );
    assert!(
        lower.contains("access-control-expose-headers: location"),
        "main exposed Location so a cross-origin browser can DELETE by it: {resp}"
    );
}

#[tokio::test]
async fn whip_options_preflight_is_204_with_cors() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = raw(
        addr,
        "OPTIONS /whip HTTP/1.1\nHost: x\nOrigin: http://example\n\
         Access-Control-Request-Method: POST\nConnection: close\n\n",
    )
    .await;
    assert!(resp.starts_with("HTTP/1.1 204"), "{resp}");
    assert!(
        resp.to_ascii_lowercase()
            .contains("access-control-allow-origin:"),
        "{resp}"
    );
}

#[tokio::test]
async fn a_body_over_64_kib_is_rejected_413_before_reading() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = raw(
        addr,
        "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\n\
         Content-Length: 131073\nConnection: close\n\n",
    )
    .await;
    assert!(resp.starts_with("HTTP/1.1 413"), "{resp}");
}

#[tokio::test]
async fn a_chunked_request_body_is_accepted() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = OFFER;
    s.write_all(
        format!(
            "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\n\
             Transfer-Encoding: chunked\nConnection: close\n\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.starts_with("HTTP/1.1 201"),
        "chunked body must be accepted: {text}"
    );
}

/// I7: the session routes must not return a success without effect — main
/// answered non-POST/non-OPTIONS with `405`.
#[tokio::test]
async fn whip_patch_and_delete_are_405_with_allow() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    for method in ["PATCH", "DELETE"] {
        let resp = raw(
            addr,
            &format!("{method} /whip/session HTTP/1.1\nHost: x\nConnection: close\n\n"),
        )
        .await;
        assert!(
            resp.starts_with("HTTP/1.1 405"),
            "{method} must be 405, not a stub 200/204: {resp}"
        );
        assert!(
            resp.to_ascii_lowercase().contains("allow:"),
            "{method} 405 must carry an Allow header: {resp}"
        );
    }
}
