# broadcast-auth 0.1.0 — 2026-07-21

First publish. A shared, scheme-agnostic HTTP/RTSP authentication crate,
extracted from `rtsp-runtime::auth` so RTSP and HTTP clients — and now
server-side output-auth gates — share one implementation instead of
duplicating it (issue #663, the multimux-hub epic).

## What it is

- **Client** ([`respond`]/[`Authenticator`]): given a `WWW-Authenticate`
  challenge received from a server, compute the `Authorization` value to
  answer it. One `Credentials` model (`Basic` / `Digest` / `Bearer`,
  `#[non_exhaustive]`) — Basic and Digest delegate to `http-auth` (RFC
  7617/7616; RTSP's reuse per RFC 2326 §14/§16), Bearer needs no challenge
  round-trip (RFC 6750). `Authenticator` holds the negotiated state across a
  session so Digest's `nc`/nonce count advances correctly on every call.
- **Server** ([`server::Verifier`]): the other half — given a configured
  `Credentials` + realm, `Verifier::challenge()` renders the
  `WWW-Authenticate` value for a `401` and `Verifier::verify(&RequestContext)
  -> AuthResult` checks an incoming request. Basic/Bearer compare in
  constant time; Digest recomputes `HA1`/`HA2`/`response` (RFC 7616 §3.4.1)
  and checks the client's claimed `uri` against the actual request URI.
  Promoted from `multimux::testutil`'s test-only mock auth server (which
  already did a real, independent Digest computation) into this shared
  crate — it is now the production verifier `multimux`'s output-auth
  middleware uses.
- **`Verifier::forwarded`** — a reverse-proxy forwarded-auth scheme: trusts
  that a fronting reverse proxy has already authenticated the caller and
  forwards the authenticated username in a configured header
  (conventionally `X-Forwarded-User`). No `Credentials`/challenge-response
  round-trip at all. **Safe only behind a trusted reverse proxy** that
  strips any client-supplied copies of that header before forwarding — see
  the `server` module docs' trust-assumption note.
- **`RequestContext`** — every request header (looked up case-insensitively
  via `RequestContext::header`) plus the transport peer address
  (`peer_addr`), attached via `with_headers`/`with_peer_addr`. This is what
  lets a server-side `Verifier` scheme see beyond `Authorization` — the
  mechanism `Verifier::forwarded` needs. `RequestContext::new` defaults both
  to empty/`None`, so a plain 2-arg client-side call site is unaffected.

## Key types

`Credentials`, `Authenticator`, `respond`, `RequestContext`,
`server::Verifier`, `server::AuthResult`, `Error`.

## Security

`Credentials`'s `Debug` never renders the raw password/token — a manual
`Debug` shows the username (not secret) for `Basic`/`Digest` and redacts
`password`/`token` as `"***"` (a pre-release audit finding: the derived
`Debug` this replaced was reachable through any embedding struct that also
derives `Debug`, e.g. `ll-hls-runtime`'s `TokioClientConfig` or
`rtsp-runtime`'s `ClientSession`).

## Compatibility

MSRV 1.86. New dependencies: `http-auth` (Basic/Digest), `base64`/`md-5`/
`rand` (server-side Digest verification — all already transitive via
`http-auth` at the same versions, so this adds no new version to the
workspace lock), `thiserror`.
