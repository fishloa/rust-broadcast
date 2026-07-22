# broadcast-auth 0.1.1 — 2026-07-22

Patch release: fixes `Verifier::verify`'s Digest check wrongly rejecting a
legitimate absolute-form request-target (issue #724).

## Fixed

`server::Verifier`'s Digest verification (RFC 7616 §3.4.1) had two related
bugs:

1. `HA2` was computed as `MD5(method:request_uri)` using the *server's*
   request URI, rather than the client's own claimed `uri` field — the
   `digest-uri-value` RFC 7616 §3.4.1 actually defines `HA2` over. A client
   whose challenge round-trip legitimately hashed a different-but-equivalent
   request-target (e.g. the absolute-form URL) could never produce a
   `response` this recomputation would match, even with the correct
   password.
2. The RFC 7616 §3.4.1 SHOULD uri-match was a strict `get("uri") !=
   request_uri` reject — rejecting the legal absolute-form representation
   outright rather than recognising it as the same request-target (RFC 7230
   §5.3.2 permits a request-target in either origin-form, `/path[?query]`,
   or absolute-form, `scheme://authority/path[?query]`).

This wrongly rejected `multimux`'s own outbound HTTP client
(`source::http_auth::authenticated_get`), which sends the absolute URL as
`uri` — a form this crate's own `RequestContext` docs already permitted, but
`Verifier::verify` couldn't actually accept.

`HA2` now uses the client's own `uri` field; the SHOULD-check is now a new
crate-internal `digest_uri_matches` helper, which accepts either
representation of the same request-target while still rejecting a
genuinely different `uri` (in either form) exactly as before. The
constant-time response comparison, and every other existing rejection path
(wrong username/realm/nonce/password, oversized header, missing fields), is
unchanged.

## Compatibility

No API changes. MSRV 1.86.
