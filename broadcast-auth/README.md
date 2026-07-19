# broadcast-auth

[![crates.io](https://img.shields.io/crates/v/broadcast-auth.svg)](https://crates.io/crates/broadcast-auth)
[![docs.rs](https://img.shields.io/docsrs/broadcast-auth)](https://docs.rs/broadcast-auth)

Shared multi-scheme authentication for RTSP and HTTP clients: one
[`Credentials`] model — **Basic**, **Digest**, **Bearer** — and one
challenge->response helper, so every credentialed client in the workspace
(`rtsp-runtime`, and `multimux`'s HTTP input adapters) answers a
`WWW-Authenticate` challenge through the same code instead of re-implementing
auth per client.

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

`Credentials::new(user, pass)` doesn't commit to Basic or Digest: the
responder answers whichever scheme the server's challenge actually advertises
(`http-auth`'s challenge parser decides the wire scheme from the challenge
text, not from the `Credentials` variant).

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
- `multimux`'s HTTP input adapters (planned) — TS-over-HTTP, HLS-pull.

[`http-auth`]: https://crates.io/crates/http-auth
