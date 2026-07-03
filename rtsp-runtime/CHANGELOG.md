# Changelog

All notable changes to `rtsp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0]

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

### Unreleased / planned

- A `tokio` (+ `tls` for `rtsps://`) socket adapter driving real connections over
  this same sans-IO core. The `tokio` and `tls` Cargo features are declared but
  currently carry no adapter code.

[0.1.0]: https://github.com/fishloa/rust-broadcast/releases
