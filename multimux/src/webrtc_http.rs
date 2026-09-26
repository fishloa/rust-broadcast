//! Bounded HTTP/1.1 request reader shared by the two hand-rolled WHIP/WHEP
//! signalling listeners (`crate::source::whip`, `crate::output::whep`).
//!
//! Both protocols need exactly the same "read one POST" shape (no chunked
//! transfer-encoding, no persistent-connection reuse — a WHIP/WHEP client's
//! POST is a single request/response) and previously kept byte-identical
//! copies of it (issue r07-C11): an unterminated header block or an
//! unbounded `Content-Length` grew `buf` without limit, and neither read had
//! any timeout, so one slow-loris connection per protocol held a task and
//! its memory open indefinitely. This is the one implementation both now
//! depend on, so the caps below cannot drift between the two again.

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// Header block cap: a request whose headers never terminate is rejected
/// (431) rather than grown without bound.
pub(crate) const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
/// Body cap, checked against the declared `Content-Length` before any body
/// bytes are read — a request claiming a larger body is rejected (413)
/// without ever reading it.
pub(crate) const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;
/// Bounds the entire read (headers + body) — a peer that stops sending
/// mid-request is closed rather than held open indefinitely.
pub(crate) const HTTP_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Why [`read_http_request`] stopped short of a parsed request. Every
/// variant means "close the connection"; callers pick the response (if any)
/// each warrants.
pub(crate) enum ReadRequestError {
    /// The socket read itself failed.
    Io(std::io::Error),
    /// [`HTTP_READ_TIMEOUT`] elapsed before the request finished arriving.
    Timeout,
    /// The header block exceeded [`MAX_HTTP_HEADER_BYTES`] before a
    /// terminating `\r\n\r\n` was seen — answer `431`.
    HeadersTooLarge,
    /// The declared `Content-Length` exceeded [`MAX_HTTP_BODY_BYTES`] —
    /// answer `413`, without reading any body bytes.
    BodyTooLarge,
}

/// One read HTTP/1.1 request: the request line, its headers in wire order
/// (name/value, whitespace-trimmed, name case as sent), and the body.
/// `headers` exists so a caller can build a `broadcast_auth::RequestContext`
/// (WHEP's output-auth gate) without this module needing to know anything
/// about that — `read_http_request_inner` already has to walk the header
/// block to find `Content-Length`, so collecting the rest costs nothing
/// extra.
pub(crate) struct HttpRequest {
    pub(crate) request_line: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Reads one HTTP/1.1 request off `stream`, bounded by
/// [`HTTP_READ_TIMEOUT`]/[`MAX_HTTP_HEADER_BYTES`]/[`MAX_HTTP_BODY_BYTES`].
/// An immediate EOF (peer closed before sending anything) is not an error —
/// it comes back as an empty [`HttpRequest`], matching the pre-existing
/// WHIP/WHEP behavior of just dropping such a connection.
pub(crate) async fn read_http_request(
    stream: &mut TcpStream,
) -> Result<HttpRequest, ReadRequestError> {
    read_http_request_with_timeout(stream, HTTP_READ_TIMEOUT).await
}

/// [`read_http_request`], with the read-bound as a parameter rather than
/// baked to [`HTTP_READ_TIMEOUT`] — exists so a test can prove the timeout
/// actually bounds the read using a short *real* duration instead of
/// `tokio::time::pause` (this crate has no `tokio` `test-util` dependency to
/// enable, and virtual time under `pause` never overlaps a genuine blocked
/// socket read the same way real time does).
pub(crate) async fn read_http_request_with_timeout(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<HttpRequest, ReadRequestError> {
    match tokio::time::timeout(timeout, read_http_request_inner(stream)).await {
        Ok(result) => result,
        Err(_) => Err(ReadRequestError::Timeout),
    }
}

async fn read_http_request_inner(stream: &mut TcpStream) -> Result<HttpRequest, ReadRequestError> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut tmp).await.map_err(ReadRequestError::Io)?;
        if n == 0 {
            return Ok(HttpRequest {
                request_line: String::new(),
                headers: Vec::new(),
                body: Vec::new(),
            });
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HTTP_HEADER_BYTES {
            return Err(ReadRequestError::HeadersTooLarge);
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    let content_length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(ReadRequestError::BodyTooLarge);
    }
    let body_start = header_end + 4;
    while buf.len() < body_start + content_length {
        let n = stream.read(&mut tmp).await.map_err(ReadRequestError::Io)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body_end = (body_start + content_length).min(buf.len());
    Ok(HttpRequest {
        request_line,
        headers,
        body: buf[body_start..body_end].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    async fn loopback_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    /// PRE-FIX FAILURE OBSERVED (against the old per-file `read_http_request`
    /// copies, no cap/timeout at all): reading a header block that never
    /// terminates would simply hang forever waiting for more bytes. Uses
    /// [`read_http_request_with_timeout`] with a short real bound (200ms)
    /// rather than [`HTTP_READ_TIMEOUT`] itself (10s) so the test stays
    /// fast — the value bounded is the same code path either way.
    #[tokio::test]
    async fn headers_that_never_terminate_time_out() {
        let (mut client, mut server) = loopback_pair().await;
        tokio::spawn(async move {
            let _ = client.write_all(b"POST / HTTP/1.1\r\nHost: x\r\n").await;
            // Never sends the terminating `\r\n\r\n`.
            std::future::pending::<()>().await;
        });
        let result = read_http_request_with_timeout(&mut server, Duration::from_millis(200)).await;
        assert!(
            matches!(result, Err(ReadRequestError::Timeout)),
            "an unterminated header block must be closed after the read timeout"
        );
    }

    /// A `Content-Length` far larger than [`MAX_HTTP_BODY_BYTES`] is
    /// rejected as soon as the headers are parsed — no body bytes are read
    /// at all, so the connection never grows memory to match the claim.
    #[tokio::test]
    async fn oversized_content_length_is_rejected_before_reading_the_body() {
        let (mut client, mut server) = loopback_pair().await;
        tokio::spawn(async move {
            let _ = client
                .write_all(b"POST / HTTP/1.1\r\nContent-Length: 10000000000\r\n\r\n")
                .await;
            // No body follows; if the reader tried to read one, it would
            // hang here rather than returning `BodyTooLarge` immediately.
        });
        let result = read_http_request(&mut server).await;
        assert!(matches!(result, Err(ReadRequestError::BodyTooLarge)));
    }

    /// An ordinary small request still round-trips, headers included.
    #[tokio::test]
    async fn ordinary_request_is_read_in_full() {
        let (mut client, mut server) = loopback_pair().await;
        tokio::spawn(async move {
            let _ = client
                .write_all(
                    b"POST /whip HTTP/1.1\r\nAuthorization: Bearer x\r\nContent-Length: 5\r\n\r\nhello",
                )
                .await;
        });
        let req = read_http_request(&mut server).await.ok().unwrap();
        assert_eq!(req.request_line, "POST /whip HTTP/1.1");
        assert_eq!(req.body, b"hello");
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer x")
        );
    }
}
