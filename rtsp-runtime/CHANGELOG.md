# Changelog

All notable changes to `rtsp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **BREAKING** The tokio adapter is a `tokio_util::codec::Framed` over the sans-IO core. New `RtspTimeouts` (connect / handshake / read_idle / write; defaults 10 s / 10 s / 30 s / 10 s) and `*_with_timeouts` constructors; `AsyncRtspClient::with_stream` / `AsyncRtspServer::with_stream` keep their signatures and use the defaults. New `Error::Timeout { what }` (and `From<std::io::Error>`). `read_idle` bounds the whole frame, so a peer dripping bytes times out; on a server connection the first request is bounded by `handshake`.
- **BREAKING** Header blocks are capped at 64 KiB while no `Content-Length` is known, and request bodies at 2 MiB (was a single 2 MiB buffer cap); the client core rejects an unterminated header over 64 KiB. The cap is enforced on the bytes buffered before the header terminator by an incremental `memchr` search resumed from the last scanned offset (a bounded framing check, not protocol parsing), so `rtsp_types::Message::parse` runs once per message instead of once per read and no unterminated shape (a long run of one byte, thousands of valid header lines) can reach the 2 MiB cap or force quadratic re-parsing; a server codec re-parses a body only once it can be complete.
- **BREAKING** `Transport` (RFC 2326 §12.39) and `Session` (§12.37) are now parsed and serialized by rtsp-runtime itself (owner decision (c); `rtsp-types` mis-parses real-world values, see README "rtsp-types gaps"): one RFC 2326 §15.1 lexer, a typed model of every parameter, canonical symmetric output. `TransportSpec` / `Transport` are `#[non_exhaustive]` (use `Default` / `rtp_avp_tcp_interleaved`), `TransportSpec::mode` is now `Vec<TransportMode>` (new public enum) and `TransportSpec::extensions` preserves unknown parameters in order; new `SessionHeader`, `session_header::DEFAULT_SESSION_TIMEOUT`, `Error::SessionParse`. Input is case-insensitive, LWS-tolerant, accepts quoted values and unquoted `mode`; range errors are rejected (port > 65535, ttl/channel > 255, ssrc not 8 hex digits). `to_header_value` is now fallible (`Result<String>`, new `Error::HeaderSerialize`): a control character in a value, a non-token parameter/extension/mode name, or an invalid session id is rejected, never emitted. Every output difference from the previous release: (1) parameter order is the spec listing order, e.g. `RTP/AVP;unicast;interleaved=2-3;mode="RECORD";append` is now `RTP/AVP;unicast;interleaved=2-3;append;mode="RECORD"` (and `append` now precedes `ttl`); (2) `mode` is upper-cased and normalised: `mode=record` was `mode="record"` and is now `mode="RECORD"`, `mode="play, record"` is now `mode="PLAY,RECORD"`; (3) unknown parameters are now preserved and emitted (`RTP/AVP;unicast;x-foo=Bar;client_port=1-2` kept `x-foo=Bar` in the new output; it was dropped); (4) a bare `destination` is emitted as `destination` (it was `destination=`); ranges stay `lo-hi`, `ssrc` stays 8 upper-case hex. `Session: x;timeout=0` and a non-numeric timeout use the 60 s default (with a warning), `ClientSession::session_warnings()` exposes the most recent `MAX_SESSION_WARNINGS` (16) warnings and `session_warning_count()` the total (also `log::warn!`); a received session id is kept verbatim whatever it contains (`"weird"`, `a b`, `abc,def`, `ab"c`) except control characters, and echoed unchanged, and the keepalive interval is floored at `MIN_KEEPALIVE_INTERVAL` (1 s).
- `sdp-types` 0.1 -> 0.2 (used only by the tests; not in the public API).

### Added
- `ClientSession::{mark_activity, poll_timeout, handle_timeout}`: keepalive deadline (half the `Session` timeout, default 60 s per RFC 2326 §12.37) driven by the adapter (`recv_interleaved` sends the `GET_PARAMETER`). Only requests written count as activity, so a busy interleaved stream does not postpone the keepalive. `ClientSession::has_buffered_input`; `recv_interleaved` now returns an error (not a clean end) when the peer closes mid-frame.
- `ClientSession::peek_next_cseq()`: the `CSeq` the next request-builder call will assign, so an
  IO adapter can capture which response it must wait for before building the request (#1088).
- `ServerEvent::MediaData`: an interleaved `$`-framed block (RFC 2326 §10.12) received by a
  server, surfaced instead of an error (see Fixed; the enum is `#[non_exhaustive]`, so this is
  additive).

### Fixed
- `WWW-Authenticate` `stale=true` was missed when `stale` was the first parameter or the realm contained a comma; `Session` header parsing is now rtsp-runtime's own RFC 2326 §12.37 parser (see Changed).
- `AsyncRtspServer::next_request` is cancel-safe on the response write: the events are kept and the response is flushed exactly once by the next call.
- Slow-loris and never-terminated requests no longer hold a server connection or grow memory (no awaited IO is unbounded).
- `AsyncRtspServer::next_request` no longer re-parses an unterminated header on every 8 KiB read (quadratic; the oversize-buffer test timed out on CI); it parses only once a blank line has arrived.
- `ClientSession`'s 401 auth retry (#1065) now replays the original request's body and every
  non-hop-by-hop header (`Content-Type` included), instead of an empty body with
  `Content-Length: 0`. An authenticated `ANNOUNCE` (RFC 2326 §10.3) previously lost its SDP on
  retry, so a push to any Digest- or Basic-challenging server always failed after the first 401.
- `ClientSession` now always rebuilds its Digest authenticator from a 401's challenge, instead of
  only when it had none yet or the challenge said `stale=true` (#1088). A server that rotates its
  nonce per session or per time window without setting `stale` used to sign the retry with the
  stale nonce, fail again, and be abandoned instead of adapted to. `stale` detection is also now
  case-insensitive and tolerates a quoted value (`stale="true"`).
- `ClientSession::handle_data` no longer aborts the whole call (discarding every event already
  decoded from the same buffer) on one response carrying an unknown or duplicate `CSeq`; that
  response is now skipped (#1088).
- `ClientSession`'s inbound buffer, and `io::AsyncRtspServer`'s request buffer, are now capped
  (1 MiB / 2 MiB) rather than unbounded, so an unterminated header or an unfulfilled
  `Content-Length` can't grow memory without limit (#1088).
- `io::AsyncRtspClient::exchange` now matches the response it returns against the `CSeq` it just
  sent, instead of returning whichever `Response` event was last in a decoded batch; a stray
  response for a request a previous, abandoned `exchange` call sent could otherwise be mistaken
  for the current one (#1088). Its `pending_media` buffer is also now capped, dropping the oldest
  frame once full, instead of growing without bound while waiting on a slow-to-answer request.
- `io::AsyncRtspServer::next_request` now demultiplexes an interleaved `$`-framed block (RFC 2326
  §10.12) into `ServerEvent::MediaData` (see Added), instead of erroring; a TCP-interleaved PLAY client
  sending RTCP receiver reports (ffmpeg, VLC, GStreamer all do) previously had its connection
  dropped at the first one (#1088).

## [0.7.0] - 2026-09-26

### Security
Fixes GHSA-3rw9-cq7p-4v47. Upgrade if you run `ServerSession`/`io::AsyncRtspServer` against
clients you do not control.

### Changed (breaking)
- `ServerSession::new` now takes the `Session` id source,
  `impl FnMut() -> u64 + Send + 'static`, which must be a CSPRNG (RFC 2326
  §3.4); `impl Default for ServerSession` is removed and `Debug` is now
  hand-written. `io::AsyncRtspServer::accept`/`accept_tls` supply the OS RNG
  (new optional `getrandom` dependency under the `tokio` feature).
  `with_session_seed` keeps its signature but is now `#[doc(hidden)]` and
  documented as for deterministic tests only.

### Fixed
- `ServerSession` ids are 64 random bits rendered as 16 hex digits instead of
  the fixed counter `305419896`, so separate connections no longer share ids.
- A request whose `Session` header names another id, or a
  `PLAY`/`PAUSE`/`RECORD`/`TEARDOWN` carrying none once a session exists, is
  answered `454 Session Not Found` (RFC 2326 §11.3.2, §12.37) instead of `200`.
- The SETUP reply's `Transport` header carries only the chosen (first) spec
  rather than every offered one (§12.39); `negotiated_transport()` and
  `ServerEvent::SessionSetup` hold that single spec.
- An unparseable `Transport` header on SETUP is answered `461 Unsupported
  Transport` instead of returning `Err` (which dropped the connection).

## [0.6.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Fixed
- Doc accuracy (#941 row 6): README install snippet corrected from `"0.3"`
  to `"0.5"` (the crate is 0.5.0).

## [0.5.0] - 2026-08-07

### Added
- `ClientSession::announce()` — ANNOUNCE request builder (RFC 2326 §10.3) (#744).
- `ClientSession::record()` — RECORD request builder (RFC 2326 §10.11) (#744).
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.3.0] - 2026-07-21

### Changed
- **Internal:** `auth` now delegates to the new shared [`broadcast-auth`](../broadcast-auth)
  crate instead of wrapping `http-auth` directly. `rtsp_runtime::Credentials`/
  `Authenticator` are re-exported unchanged (same `Credentials::new(user, pass)`
  API, same transparent-401-retry behaviour) — this is a refactor with no
  behaviour change, extracted so RTSP and (future) HTTP clients share one auth
  implementation instead of duplicating it. Adds `Credentials::bearer(token)`
  (RFC 6750), previously unsupported (#663 multimux-hub P3b).

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

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

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
