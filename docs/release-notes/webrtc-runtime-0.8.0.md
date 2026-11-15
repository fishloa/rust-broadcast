# webrtc-runtime 0.8.0

_Released 2026-11-15._

Minor-class release (0.7 -> 0.8). The WHIP/WHEP client takes the workspace's one auth vocabulary (`broadcast_auth::Credentials`) instead of a bearer-only string, and the media events and errors that carried free text now carry typed values, so a host can branch on an ICE state or a media failure without parsing a message.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

### Auth is `Credentials`

`WebRtcClientConfig::bearer_token: Option<String>`/`with_bearer_token` are `credentials: Option<broadcast_auth::Credentials>`/`with_credentials(Credentials)`, the `rtsp-runtime` shape: one auth vocabulary across the runtimes. Bearer is applied as before; Basic is applied pre-emptively (a WHIP endpoint behind a proxy); the redacting `Debug` now hides `credentials`.

```rust
// before
let config = WebRtcClientConfig::new(endpoint).with_bearer_token(token);
// after
let config = WebRtcClientConfig::new(endpoint).with_credentials(broadcast_auth::Credentials::Bearer { token });
```

### The client config is `WebRtcClientConfig`

`session::client::ClientConfig` is `session::client::WebRtcClientConfig` (the crate no longer names a type `ClientConfig`), and its constructor takes any string-like endpoint.

```rust
// before
let config = session::client::ClientConfig::new(url.into());
// after
let config = session::client::WebRtcClientConfig::new(url);
```

### The media events and errors are typed

`MediaEvent::IceStateChanged` carries an `IceConnectionState` (was a `String`), `LocalCandidateGathered` carries an `IceCandidate`, `AuthFailure`/`TimerError` carry a `MediaError`, `Error::Media(String)` is `Media(#[from] MediaError)`, and `DecryptedRtp.payload`/extension data are `bytes::Bytes`.

```rust
// before
if let MediaEvent::IceStateChanged(ref state) = event { if state == "connected" { .. } }
if let Error::Media(ref message) = err { eprintln!("{message}"); }
// after
if let MediaEvent::IceStateChanged(state) = event { if state == IceConnectionState::Connected { .. } }
if let Error::Media(source) = err { eprintln!("{source}"); }
```

### The published types are `#[non_exhaustive]`

`media::MediaTransportConfig`, `HttpRequest` and `HttpResponse` are `#[non_exhaustive]`; build them with `MediaTransportConfig::new(..)`/`HttpRequest::new(..)`/`HttpResponse::new(..)` + `with_*` instead of struct literals.

```rust
// before
let request = HttpRequest { method, url, headers, body };
// after
let request = HttpRequest::new(method, url).with_headers(headers).with_body(body);
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `broadcast-auth` 0.7 -> 0.8, `rtp-packet` 0.7 -> 0.8.

---

Published from tag `webrtc-runtime-v0.8.0`.
