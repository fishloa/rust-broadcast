# Changelog

All notable changes to `rtsp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


### Added
- Pre-release hardening (release audit): `tests/label_coverage.rs` #204 drift-guard
  (SessionState/LowerTransport/Delivery labelled; Error/ClientEvent/ServerEvent skipped),
  named `DEFAULT_SESSION_SEED`, and field-mutation round-trip bites for Transport /
  InterleavedFrame.

- **Async socket adapter** (feature `tokio`) — `io::AsyncRtspClient` /
  `io::AsyncRtspServer` (RFC 2326 transport). Each owns a
  `tokio::net::TcpStream`, writes the request/response bytes the sans-IO session
  produces, reads the peer's reply — buffering fragmented reads until a full
  RTSP message or interleaved `$`-frame parses (§10.12) — feeds it through
  `handle_data`/`handle_request`, and returns the `ClientEvent`/`ServerEvent`s.
  The client answers Digest `401` challenges transparently and pulls interleaved
  media with `recv_interleaved`; the server sends media with `send_interleaved`.
  Both are generic over the stream type (`AsyncRead + AsyncWrite + Unpin`).
- **`rtsps://` over TLS** (feature `tls`) — `AsyncRtspClient::connect_tls` /
  `AsyncRtspServer::accept_tls` wrap the TCP stream in a `tokio-rustls` session
  before the RTSP exchange (default port **322**). `default_tls_client_config`
  builds a `rustls::ClientConfig` trusting the `webpki-roots` bundle; a custom
  config can trust a self-signed camera cert.
- New `Error::Io` / `Error::Tls` variants for the async adapter's failure
  surface.

## [0.1.0] - 2026-07-03

Initial release — the sans-IO RTSP 1.0 ([RFC 2326](https://www.rfc-editor.org/rfc/rfc2326))
session engine.

### Added

- **Sans-IO session engine.** Feed inbound bytes and read back outbound bytes +
  typed events; no sockets in the core.
- **Client (`ClientSession`)** — RFC 2326 Appendix A.1. Request builders
  (`options`, `describe`, `setup`, `play`, `pause`, `teardown`, `get_parameter`)
  that reject any method not valid in the current state, attach an incrementing
  `CSeq`, the negotiated `Session` id, and (once authenticated) an
  `Authorization` header. `handle_data` correlates responses by `CSeq`, advances
  the state machine on `2xx`, resets to `Init` on `3xx`, transparently answers
  `401` challenges (including `stale=true` nonce refresh), captures the `Session`
  id/timeout from the SETUP response, and surfaces interleaved frames as
  `ClientEvent::MediaData`.
- **Server (`ServerSession`)** — RFC 2326 Appendix A.2. `handle_request` returns
  the response bytes plus events, validates the method against the server state
  table (`455 Method Not Valid In This State` otherwise), allocates a `Session`
  id on SETUP, and echoes/negotiates the `Transport` (`461 Unsupported
  Transport` when nothing is offered).
- **`Transport` header** (§12.39) — a typed, round-trippable `Transport` /
  `TransportSpec` supporting `RTP/AVP[/TCP|/UDP]`, `unicast`/`multicast`,
  `interleaved`, `client_port`/`server_port`/`port`, `ttl`, `layers`, `ssrc`,
  `destination`/`source`, `mode`, and `append`.
- **Interleaved framing** (§10.12) — `InterleavedFrame` parse/serialize plus a
  streaming demultiplexer (`interleaved::parse_frames`) that returns the complete
  frames and the unconsumed partial-tail length.
- **Authentication** (§14) — Basic and Digest wired over the `http-auth` crate
  via `Credentials` / `Authenticator`, using the RTSP request URI in the digest
  computation.
- **Structured errors** (`Error`) and an optional `serde` feature on the public
  wire/event types.
- Message parse/serialize delegated to `rtsp-types`; SDP to `sdp-types`.

[0.1.0]: https://github.com/fishloa/rust-broadcast/releases
