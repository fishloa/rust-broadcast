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
#[derive(Clone, Copy)]
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

/// Manual `Debug` (rather than `#[derive(Debug)]`): [`Self::headers`] carries
/// whatever the caller attached, which — server-side — includes the real
/// `Authorization`/`Proxy-Authorization` header the request was authenticated
/// with. Basic's value is a reversible base64 `user:pass` (RFC 7617 §2); a
/// bare `tracing::debug!(?ctx, ...)` call must not dump it to logs. Every
/// other header (name and value) is rendered normally — only the value of an
/// auth header is redacted, and only that header's name is enough to tell
/// which one.
impl core::fmt::Debug for RequestContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        struct Headers<'a>(&'a [(&'a str, &'a str)]);
        impl core::fmt::Debug for Headers<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_list()
                    .entries(self.0.iter().map(|(name, value)| {
                        let redact = name.eq_ignore_ascii_case("authorization")
                            || name.eq_ignore_ascii_case("proxy-authorization");
                        (*name, if redact { "<redacted>" } else { value })
                    }))
                    .finish()
            }
        }
        f.debug_struct("RequestContext")
            .field("method", &self.method)
            .field("uri", &self.uri)
            .field("body", &self.body)
            .field("headers", &Headers(self.headers))
            .field("peer_addr", &self.peer_addr)
            .finish()
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

    // Security-blocker regression (pre-release audit): `RequestContext` must
    // never render an `Authorization`/`Proxy-Authorization` header's value
    // via `{:?}` — a bare `tracing::debug!(?ctx, ...)` must not leak
    // credentials to logs. Must FAIL if `Debug` reverts to a plain
    // `#[derive(Debug)]`.
    #[test]
    fn debug_redacts_authorization_header_value_but_keeps_other_fields() {
        // "admin:hunter2-super-secret" base64-encoded.
        let auth_value = "Basic YWRtaW46aHVudGVyMi1zdXBlci1zZWNyZXQ=";
        let headers: &[(&str, &str)] =
            &[("Authorization", auth_value), ("X-Forwarded-User", "alice")];
        let ctx = RequestContext::new("DESCRIBE", "rtsp://cam/live").with_headers(headers);
        let debug = format!("{ctx:?}");

        assert!(
            !debug.contains("YWRtaW46aHVudGVyMi1zdXBlci1zZWNyZXQ"),
            "leaked base64 secret: {debug}"
        );
        assert!(
            !debug.contains("hunter2"),
            "leaked password substring: {debug}"
        );
        assert!(
            debug.contains("<redacted>"),
            "expected redaction marker: {debug}"
        );

        // Non-secret fields/headers must still be visible for diagnostics.
        assert!(debug.contains("DESCRIBE"), "method missing: {debug}");
        assert!(debug.contains("rtsp://cam/live"), "uri missing: {debug}");
        assert!(
            debug.contains("Authorization"),
            "header name should still be shown: {debug}"
        );
        assert!(
            debug.contains("X-Forwarded-User") && debug.contains("alice"),
            "non-secret header should render normally: {debug}"
        );
    }

    // Same regression, case-insensitively, for the proxy variant (RFC 7235
    // §4.4) and for `Proxy-Authorization` sent in a non-canonical case.
    #[test]
    fn debug_redacts_proxy_authorization_header_case_insensitively() {
        let headers: &[(&str, &str)] = &[("proxy-AUTHORIZATION", "Basic c2VjcmV0LXBhc3N3b3Jk")];
        let ctx = RequestContext::new("GET", "/x").with_headers(headers);
        let debug = format!("{ctx:?}");
        assert!(
            !debug.contains("c2VjcmV0LXBhc3N3b3Jk"),
            "leaked base64 secret: {debug}"
        );
        assert!(
            debug.contains("<redacted>"),
            "expected redaction marker: {debug}"
        );
    }
}
