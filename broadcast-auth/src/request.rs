//! The request context an `Authorization` response is computed against.

/// The request fields the Digest response hash covers (RFC 7616 §3.4.1 / RFC
/// 2326 §14): the method, the request URI, and — for `qop=auth-int` — the
/// body.
///
/// The `uri` is scheme-specific: an HTTP absolute/relative URL for HTTP
/// clients, or the RTSP request URI (e.g. `rtsp://host/stream`) for RTSP —
/// never translate one into the other (RFC 2326 §14).
#[derive(Debug, Clone, Copy)]
pub struct RequestContext<'a> {
    /// The request method (`"GET"`, `"DESCRIBE"`, …).
    pub method: &'a str,
    /// The request URI, in the caller's protocol's own form.
    pub uri: &'a str,
    /// The request body, needed only for Digest `qop=auth-int`. `Some(&[])`
    /// for a bodyless request still lets `auth-int` be computed.
    pub body: Option<&'a [u8]>,
}

impl<'a> RequestContext<'a> {
    /// Builds a context for a bodyless request (the common case).
    pub fn new(method: &'a str, uri: &'a str) -> Self {
        RequestContext {
            method,
            uri,
            body: Some(&[]),
        }
    }

    /// Attaches a request body (for `qop=auth-int`).
    pub fn with_body(mut self, body: &'a [u8]) -> Self {
        self.body = Some(body);
        self
    }
}
