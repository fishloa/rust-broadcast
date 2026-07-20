//! The request context an `Authorization` response is computed against — and,
//! server-side, that a [`crate::Verifier`] checks a request against.

/// The request fields the Digest response hash covers (RFC 7616 §3.4.1 / RFC
/// 2326 §14): the method, the request URI, and — for `qop=auth-int` — the
/// body. Also carries the request's headers and transport peer address, so a
/// server-side [`crate::Verifier`] scheme can see beyond the `Authorization`
/// header — e.g. a reverse-proxy forwarded-auth scheme reading
/// `X-Forwarded-User`/`X-Forwarded-For` (issue #663 extensibility wave part
/// 1). Client-side use ([`crate::respond`]/[`crate::Authenticator`]) needs
/// neither field; [`Self::new`] defaults both to empty/`None`.
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
    /// Every request header, as `(name, value)` pairs — [`Self::header`]
    /// looks one up case-insensitively (header names are case-insensitive,
    /// RFC 7230 §3.2). Empty for client-side use (`Self::new`'s default): a
    /// client answering a challenge computes `Authorization` from
    /// `method`/`uri`/`body` alone. Server-side ([`crate::Verifier::verify`])
    /// this is how every scheme — including a future one — reads whatever
    /// header it needs, not just `Authorization`.
    pub headers: &'a [(&'a str, &'a str)],
    /// The transport-layer peer address (e.g. the accepted TCP connection's
    /// remote address), if the caller has one to attach. This is the actual
    /// connection peer — which, behind a reverse proxy, is the proxy itself,
    /// not the original client (see `X-Forwarded-For` in [`Self::headers`]
    /// for that). `None` for client-side use and whenever the caller has no
    /// transport peer to attach.
    pub peer_addr: Option<std::net::SocketAddr>,
}

impl<'a> RequestContext<'a> {
    /// Builds a context for a bodyless request with no headers/peer attached
    /// (the common client-side case) — use [`Self::with_headers`]/
    /// [`Self::with_peer_addr`] to attach either.
    pub fn new(method: &'a str, uri: &'a str) -> Self {
        RequestContext {
            method,
            uri,
            body: Some(&[]),
            headers: &[],
            peer_addr: None,
        }
    }

    /// Attaches a request body (for `qop=auth-int`).
    pub fn with_body(mut self, body: &'a [u8]) -> Self {
        self.body = Some(body);
        self
    }

    /// Attaches the request's headers (server-side use — see
    /// [`Self::headers`]).
    pub fn with_headers(mut self, headers: &'a [(&'a str, &'a str)]) -> Self {
        self.headers = headers;
        self
    }

    /// Attaches the transport peer address (server-side use — see
    /// [`Self::peer_addr`]).
    pub fn with_peer_addr(mut self, peer_addr: std::net::SocketAddr) -> Self {
        self.peer_addr = Some(peer_addr);
        self
    }

    /// Looks up a header by name, case-insensitively (RFC 7230 §3.2). Returns
    /// the first match if [`Self::headers`] carries more than one with the
    /// same name.
    pub fn header(&self, name: &str) -> Option<&'a str> {
        for &(k, v) in self.headers {
            if k.eq_ignore_ascii_case(name) {
                return Some(v);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_lookup_is_case_insensitive() {
        let headers: &[(&str, &str)] = &[("X-Forwarded-User", "alice")];
        let ctx = RequestContext::new("GET", "/x").with_headers(headers);
        assert_eq!(ctx.header("x-forwarded-user"), Some("alice"));
        assert_eq!(ctx.header("X-FORWARDED-USER"), Some("alice"));
        assert_eq!(ctx.header("x-forwarded-for"), None);
    }

    #[test]
    fn new_defaults_to_no_headers_and_no_peer() {
        let ctx = RequestContext::new("GET", "/x");
        assert_eq!(ctx.header("authorization"), None);
        assert_eq!(ctx.peer_addr, None);
    }

    #[test]
    fn with_peer_addr_round_trips() {
        let addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let ctx = RequestContext::new("GET", "/x").with_peer_addr(addr);
        assert_eq!(ctx.peer_addr, Some(addr));
    }
}
