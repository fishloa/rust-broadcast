# broadcast-auth

[![crates.io](https://img.shields.io/crates/v/broadcast-auth.svg)](https://crates.io/crates/broadcast-auth)
[![docs.rs](https://img.shields.io/docsrs/broadcast-auth)](https://docs.rs/broadcast-auth)

Shared multi-scheme authentication for RTSP and HTTP clients **and
servers**: one [`Credentials`] model — **Basic**, **Digest**, **Bearer** —
one client-side challenge->response helper, and one server-side
challenge+verify type (`Verifier`, plus a reverse-proxy **Forwarded**
scheme), so every credentialed client and server in the workspace
(`rtsp-runtime`; `multimux`'s HTTP input adapters *and* its shared
output-auth gate) answers/issues a `WWW-Authenticate` challenge through the
same code instead of re-implementing auth per client or per origin.

## Why

Auth is not transport-specific. RTSP (RFC 2326 §14/§16 reuses HTTP's Basic and
Digest verbatim), TS-over-HTTP, HLS-pull, and any other credentialed origin all
face the same handful of schemes. Before this crate, `rtsp-runtime` carried its
own `Credentials`/`Authenticator` pair wrapping [`http-auth`]; this crate pulls
that logic out so it can be shared, and adds Bearer (RFC 6750), which
`http-auth` does not cover (Bearer needs no challenge-response round-trip at
all).

## Schemes

- **Basic** (RFC 7617) / **Digest** (RFC 7616) — challenge parsing and the
  Digest response hash are delegated to the mature [`http-auth`] crate.
  `Authenticator` keeps the negotiated state alive across a session so
  Digest's `nc` (nonce count) advances correctly on every subsequent request.
- **Bearer** (RFC 6750) — no challenge needed; the `Authorization` value is
  always `Bearer <token>`.
- **Forwarded** (server-side only) — trusts that a fronting reverse proxy
  has already authenticated the caller and forwards the authenticated
  username in a configured header (conventionally `X-Forwarded-User`); no
  `Credentials`/challenge-response round-trip at all. **Safe only behind a
  trusted reverse proxy** that strips any client-supplied copy of that
  header before forwarding — see `Verifier::forwarded`'s doc comment.

`Credentials::new(user, pass)` doesn't commit to Basic or Digest: the
responder answers whichever scheme the server's challenge actually advertises
(`http-auth`'s challenge parser decides the wire scheme from the challenge
text, not from the `Credentials` variant).

## Client and server

- **Client** (`respond`/`Authenticator`): given a `WWW-Authenticate`
  challenge received from a server, compute the `Authorization` value to
  answer it — see Usage below.
- **Server** (`server::Verifier`): given a configured `Credentials` + realm,
  `Verifier::challenge()` renders the `WWW-Authenticate` value for a `401`
  and `Verifier::verify(&RequestContext)` checks an incoming request.
  Basic/Bearer compare in constant time; Digest recomputes the response hash
  (RFC 7616 §3.4.1) and also checks the client's claimed `uri` against the
  actual request URI. This is the production verifier behind `multimux`'s
  shared output-auth gate.
- **`RequestContext`** carries the method/URI/body (needed to compute or
  verify a response) plus, for server-side use, every request header
  (`headers`, looked up case-insensitively via `RequestContext::header`) and
  the transport peer address (`peer_addr`) — what lets a `Verifier` scheme
  see beyond `Authorization` (the mechanism `Verifier::forwarded` needs).
  Both default to empty/`None` via `RequestContext::new`, so a client-side
  2-arg call site is unaffected.

## Usage

```rust
use broadcast_auth::{Authenticator, Credentials, RequestContext};

// Negotiate once from the server's 401 WWW-Authenticate value...
let mut auth = Authenticator::from_challenge(
    "Digest realm=\"cameras\", nonce=\"abc123\", qop=\"auth\"",
    Credentials::new("admin", "12345"),
)?;

// ...then answer every subsequent request with the same Authenticator so
// Digest's nc advances (RFC 7616 §3.3):
let value = auth.authorization(&RequestContext::new("DESCRIBE", "rtsp://cam/stream"))?;
assert!(value.starts_with("Digest "));
# Ok::<(), broadcast_auth::Error>(())
```

Bearer skips the challenge entirely:

```rust
use broadcast_auth::Credentials;
let creds = Credentials::bearer("mytoken");
```

A one-shot helper (`respond`) is available for callers that don't need to keep
an `Authenticator` around for more than one request.

## Scope

In: the `Credentials` model, challenge parsing (via `http-auth`) + response
computation for Basic/Digest, Bearer's stateless header value, a stateful
`Authenticator` for session reuse.

Out: transport IO (sockets, header framing) — that's each client's own job
(`rtsp-runtime`'s session engine, `multimux`'s HTTP adapters). Out: credential
*storage*/config parsing — callers build `Credentials` from URL userinfo or
their own config.

## Consumers

- [`rtsp-runtime`](../rtsp-runtime) — re-exports `broadcast_auth::Credentials`
  as `rtsp_runtime::Credentials` and delegates its `Authenticator` to this
  crate (RFC 2326 §14).
- [`ll-hls-runtime`](../ll-hls-runtime) — `client::tokio_client::TokioClient`
  authenticates via this crate's `Credentials`/`Authenticator`
  (Basic/Digest/Bearer).
- `multimux` — client-side: its `TsHttp`/`HlsPull`/`Rtsp` input adapters
  (`source::http_auth`) answer upstream challenges via this crate's
  `Authenticator`. Server-side: `Config::output_auth`'s shared output-auth
  gate (every `/{stream}/…` route, across every configured route) is built
  on this crate's `server::Verifier`, including the `Forwarded` scheme for a
  reverse-proxy deployment.

[`http-auth`]: https://crates.io/crates/http-auth
