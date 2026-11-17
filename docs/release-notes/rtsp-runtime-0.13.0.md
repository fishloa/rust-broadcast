# rtsp-runtime 0.13.0

_Released 2026-11-17._

Minor-class release (0.12 -> 0.13) from the audit 20 bounds bundle. The async client keeps one read deadline across an inbound read, `ClientSession` holds at most 64 unanswered requests, and header values are checked before serialisation. TLS moves to the ring provider, and `bytes/serde` is no longer linked unless the crate's `serde` feature is on.

## Upgrading

- BREAKING: TLS uses the ring crypto provider; `rustls`/`tokio-rustls` are built without default features, so aws-lc-rs and aws-lc-sys leave the graph. A caller that needs aws-lc-rs must select it with `ClientConfig::builder_with_provider`.
- BREAKING: `ClientSession::with_user_agent`, `RtspClientConfig::with_user_agent` and `RtspServerConfig::with_server_header` return `Result<Self>`; add `?`. A header value with a control character is rejected as `Error::HeaderSerialize` instead of being written raw.

## Added

- `ServerCodec::decode` is pinned at its message-size cap: a `Content-Length` of 3 000 000 bytes is refused with `MessageParseError::TooLarge`, a message of exactly `MAX_MESSAGE_BYTES` is still a body wait, and one byte over is refused (#2860).

## Changed

- BREAKING: the TLS crypto provider is ring, not aws-lc-rs. `io::default_tls_client_config` and the `tls` feature's `rustls` dependency build on `rustls::crypto::ring`, and `rustls`/`tokio-rustls` are taken with `default-features = false`, so aws-lc-rs and aws-lc-sys leave the build graph (#2877). Migration: a caller that needs the aws-lc-rs provider must select it itself with `ClientConfig::builder_with_provider`.
- `bytes` no longer turns on its own `serde` feature unconditionally: only this crate's opt-in `serde` feature forwards `bytes/serde`, so a build without it no longer links `serde` (#2872).
- BREAKING: `ClientSession::with_user_agent`, `RtspClientConfig::with_user_agent` and `RtspServerConfig::with_server_header` return `Result<Self>`, rejecting a value with a control character as `Error::HeaderSerialize`. `rtsp-types` stores a header value as a bare `String` and the serializer writes it raw, so a CR/LF in config forged a header or ended the header block. Migration: `.with_user_agent(ua)` -> `.with_user_agent(ua)?` (likewise `with_server_header`) (#2859).

## Fixed

- The async client keeps one read deadline for the whole inbound read and pushes it back only when a frame is decoded, instead of restarting `Timeouts::read_idle` on every call. A keepalive wake in `recv_interleaved` at half the session timeout (always shorter than `read_idle` by default) used to cancel and restart the read timer, so a server that answered nothing was never detected (#2838).
- `ClientSession` remembers at most `MAX_PENDING_REQUESTS` (64) unanswered requests, evicting the oldest `CSeq`. A server withholding replies used to leak one full request copy per keepalive interval and per abandoned exchange, without bound (#2839).

---

Published from tag `rtsp-runtime-v0.13.0`.
