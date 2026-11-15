# broadcast-auth 0.8.0

_Released 2026-11-15._

### Changed
- BREAKING: the string errors are typed (#2734): `Error::InvalidTrustedProxy(String)` is a struct variant `InvalidTrustedProxy { input, kind: TrustedProxyFailure }` (`NotIpAddress`/`PrefixOutOfRange`), `Error::UnknownSignedUrlKeyId(String)` is `UnknownSignedUrlKeyId { kid }`, and `Error::InvalidUserinfo(String)` is a unit variant (the percent-decode failure has one mode, so no reason is carried). A caller branches on the typed kind rather than the message text.
- BREAKING: `AuthState::handle_unauthorized` takes `status: http::StatusCode` and `&HeaderMap` (was `u16` and `&[(String, String)]`), `AuthState::receive_cookies` takes `&HeaderMap`, `AuthEvent::AuthRejected { status }` is a `http::StatusCode`, and `carries_credentials` takes `&HeaderMap`. The `Authorization`/`WWW-Authenticate` values come from the response's own header map instead of hand-built pairs (#2692).

- BREAKING: `RequestContext` is now `#[non_exhaustive]`; build it with `RequestContext::new(method, uri)` + `with_body`/`with_headers`/`with_peer_addr` (the only external construction path, unchanged) (#2664).
- BREAKING: `AuthState::{request_headers, receive_cookies, cookies}` and `CookieWatch::cookies` take the caller's clock as a `broadcast_common::stage::Timestamp` (the type every sans-IO engine takes) instead of `now_us: u64`.
- BREAKING: `Error::ChallengeParse` and `Error::ResponseCompute` now wrap `HttpAuthError` (which implements `std::error::Error` as `#[source]`) instead of a bare `String`; construct via `HttpAuthError(...)` (#2659).

---

Published from tag `broadcast-auth-v0.8.0`.
