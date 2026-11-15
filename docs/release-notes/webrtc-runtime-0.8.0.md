# webrtc-runtime 0.8.0

_Released 2026-11-15._

### Changed
- BREAKING: `WebRtcClientConfig`'s `bearer_token: Option<String>`/`with_bearer_token` are `credentials: Option<broadcast_auth::Credentials>`/`with_credentials(Credentials)`, the rtsp-runtime shape: one auth vocabulary across the runtimes. Bearer is applied as before; Basic is applied pre-emptively (a WHIP endpoint behind a proxy); the redacting `Debug` now hides `credentials`. Migration: `.with_bearer_token(t)` -> `.with_credentials(broadcast_auth::Credentials::Bearer { token: t })` (#2661 2.2).
- BREAKING: `media::MediaTransportConfig`, `HttpRequest` and `HttpResponse` are now `#[non_exhaustive]`; build them with `MediaTransportConfig::new(..)`/`HttpRequest::new(..)`/`HttpResponse::new(..)` + `with_*` instead of struct literals (#2664).
- BREAKING: `session::client::ClientConfig` is `session::client::WebRtcClientConfig` (the crate no longer names a type `ClientConfig`), and `WebRtcClientConfig::new(endpoint_url: impl Into<String>)` takes any string-like endpoint. Migration: `ClientConfig::new(URL.into())` -> `WebRtcClientConfig::new(URL)` (#2660).
- BREAKING: `MediaEvent::IceStateChanged` now carries `IceConnectionState` instead of a `String`; `LocalCandidateGathered` carries `IceCandidate`; `AuthFailure`/`TimerError` carry `MediaError`; `Error::Media(String)` becomes `Media(#[from] MediaError)`; `DecryptedRtp.payload`/extension data use `bytes::Bytes` (#2658).

- Narrowed the `tokio` dev-dependency from `features = ["full"]` to the features the tests use (`net`, `io-util`, `rt`, `rt-multi-thread`, `macros`, `time`, `sync`).

---

Published from tag `webrtc-runtime-v0.8.0`.
