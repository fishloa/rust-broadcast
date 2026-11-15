# broadcast-auth 0.8.0

_Released 2026-11-15._

Minor-class release (0.7 -> 0.8). The verifier takes the response's own `http` header map instead of hand-built `(String, String)` pairs, the caller's clock is the workspace's `stage::Timestamp`, and the error variants that carried a message now carry a typed reason, so a host branches on the failure kind rather than its text.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

### The verifier and the client read typed HTTP values

`AuthState::handle_unauthorized` takes `status: http::StatusCode` and `&http::HeaderMap` (was `u16` and `&[(String, String)]`), `AuthState::receive_cookies` takes `&HeaderMap`, `AuthEvent::AuthRejected { status }` is a `http::StatusCode`, and `carries_credentials` takes `&HeaderMap`. The `Authorization`/`WWW-Authenticate` values come from the response's own header map instead of hand-built pairs.

```rust
// before
auth.handle_unauthorized(401, &headers, had_auth, now_us);
// after
auth.handle_unauthorized(http::StatusCode::UNAUTHORIZED, &headers, had_auth, now);
```

### The clock is a `stage::Timestamp`

`AuthState::{request_headers, receive_cookies, cookies}` and `CookieWatch::cookies` take the caller's clock as a `broadcast_common::stage::Timestamp` (the type every sans-IO engine takes) instead of `now_us: u64`.

```rust
// before
let headers = auth.request_headers(&url, now_us);
// after
let headers = auth.request_headers(&url, Timestamp::from_micros(now_us));
```

### `RequestContext` is `#[non_exhaustive]`

It is built with `RequestContext::new(method, uri)` + `with_body`/`with_headers`/`with_peer_addr` (the only external construction path, unchanged) instead of a struct literal.

```rust
// before
let ctx = RequestContext { method, uri, body, headers, peer_addr };
// after
let ctx = RequestContext::new(method, uri)
    .with_body(body)
    .with_headers(&[("Authorization", value)])
    .with_peer_addr(peer_addr);
```

### The errors are typed

`Error::InvalidTrustedProxy(String)` is a struct variant `InvalidTrustedProxy { input, kind: TrustedProxyFailure }` (`NotIpAddress`, `PrefixOutOfRange`), `Error::UnknownSignedUrlKeyId(String)` is `UnknownSignedUrlKeyId { kid }`, `Error::InvalidUserinfo(String)` is a unit variant (the percent-decode failure has one mode), and `Error::ChallengeParse`/`Error::ResponseCompute` wrap `HttpAuthError` (which implements `std::error::Error` as `#[source]`) instead of a bare `String`.

```rust
// before
if let Error::UnknownSignedUrlKeyId(ref kid) = err { eprintln!("{kid}"); }
// after
if let Error::UnknownSignedUrlKeyId { kid } = err { eprintln!("{kid}"); }
if let Error::InvalidTrustedProxy { kind: TrustedProxyFailure::NotIpAddress, .. } = err { .. }
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `broadcast-auth-v0.8.0`.
