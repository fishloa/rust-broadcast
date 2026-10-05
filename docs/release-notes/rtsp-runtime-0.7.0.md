# rtsp-runtime 0.7.0

_Released 2026-10-05._

Breaking (0.x minor), large release. It is the first publication since 0.6.0 and combines a security fix for `ServerSession` session ids with the de-hand-roll wave's rework of the tokio adapter and of `Transport`/`Session` header handling. **Upgrade if you run `ServerSession` or `io::AsyncRtspServer` against clients you do not control.** Who must act: anyone calling `ServerSession::new()` or `ServerSession::default()` (the id source is now required), anyone building or comparing `Transport` header values (typed model, different output, fallible `to_header_value`), and anyone relying on the tokio adapter having no timeouts or size limits. The latest published version before this one is 0.6.0. The crate now builds on `broadcast-auth` 0.4, whose client-side change (a Bearer token that cannot be a header value is refused) applies to the 401 retry path: see [broadcast-auth-0.4.0.md](broadcast-auth-0.4.0.md).

## Security

**GHSA-3rw9-cq7p-4v47.** Every `ServerSession` started its `Session` id counter at the same fixed value (`305419896`, `0x1234_5678`) and incremented from there, so a client of one session could predict or reuse another's id. `Session` header validation on `PLAY`, `PAUSE`, `RECORD` and `TEARDOWN` was also missing, so a request naming any id was accepted. Ids are now 64 random bits rendered as 16 hex digits, and a request whose `Session` header names another id, or a `PLAY`/`PAUSE`/`RECORD`/`TEARDOWN` naming none once a session exists, is answered `454 Session Not Found` (RFC 2326 §11.3.2, §12.37) instead of `200`.

## Breaking changes

### 1. `ServerSession::new` takes the session id source

```rust
// 0.6.x
let session = ServerSession::new();

// 0.7.0: caller supplies a CSPRNG (RFC 2326 §3.4) as `impl FnMut() -> u64 + Send + 'static`
fn os_session_id() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("OS random source unavailable");
    u64::from_ne_bytes(bytes)
}
let session = ServerSession::new(os_session_id);
```

`impl Default for ServerSession` is removed (there is no safe default id source) and `Debug` is hand-written. `with_session_seed` keeps its signature but is `#[doc(hidden)]`, for deterministic tests only. `io::AsyncRtspServer::accept`/`accept_tls` supply the OS RNG for you through a new optional `getrandom` dependency under the `tokio` feature; no change at those call sites.

### 2. `Transport` and `Session` are parsed and serialized by this crate

`rtsp-types` mis-parses real-world `Transport` (§12.39) and `Session` (§12.37) values, so rtsp-runtime now has its own RFC 2326 §15.1 lexer, a typed model of every parameter, and canonical symmetric output.

- `TransportSpec` and `Transport` are `#[non_exhaustive]` (construct with `Default` or `TransportSpec::rtp_avp_tcp_interleaved(lo, hi)`).
- `TransportSpec::mode` is `Vec<TransportMode>` (was `Option<String>`); new public enum `TransportMode { Play, Record, Other(String) }`. `TransportSpec::extensions` preserves unknown parameters in order.
- New `SessionHeader`, `session_header::DEFAULT_SESSION_TIMEOUT` (60 s), `Error::SessionParse`.
- `to_header_value` is now fallible: `Result<String>` with new `Error::HeaderSerialize`. A control character in a value, a non-token parameter, extension or mode name, or an invalid session id is rejected, never emitted.
- Input is case-insensitive, LWS-tolerant, accepts quoted values and unquoted `mode`; range errors are rejected (port > 65535, ttl or channel > 255, `ssrc` not 8 hex digits).

```rust
// 0.6.x
let v: String = transport.to_header_value();
// 0.7.0
let v: String = transport.to_header_value()?;
```

Every output difference from 0.6.x, for anyone comparing header text:

1. Parameter order is the spec listing order: `RTP/AVP;unicast;interleaved=2-3;mode="RECORD";append` is now `RTP/AVP;unicast;interleaved=2-3;append;mode="RECORD"` (and `append` now precedes `ttl`).
2. `mode` is upper-cased and normalised: `mode=record` was `mode="record"` and is now `mode="RECORD"`; `mode="play, record"` is now `mode="PLAY,RECORD"`.
3. Unknown parameters are preserved and emitted (`x-foo=Bar` used to be dropped).
4. A bare `destination` is emitted as `destination` (was `destination=`). Ranges stay `lo-hi`, `ssrc` stays 8 upper-case hex.

`Session: x;timeout=0` and a non-numeric timeout use the 60 s default with a warning: `ClientSession::session_warnings()` exposes the most recent `MAX_SESSION_WARNINGS` (16) and `session_warning_count()` the total (also `log::warn!`). A received session id is kept verbatim whatever it contains (`"weird"`, `a b`, `abc,def`) except control characters, and echoed unchanged; the keepalive interval is floored at `MIN_KEEPALIVE_INTERVAL` (1 s). An unterminated quote in a received `Session` id (`ab"c;timeout=30`) no longer swallows the `;`, so `timeout` is not lost.

### 3. Tokio adapter is a `tokio_util::codec::Framed`, with timeouts and size caps

New `io::RtspTimeouts { connect, handshake, read_idle, write }` (defaults 10 s, 10 s, 30 s, 10 s) and `*_with_timeouts` constructors (`connect_with_timeouts`, `connect_tls_with_timeouts`, `accept_tls_with_timeouts`). `AsyncRtspClient::with_stream` and `AsyncRtspServer::with_stream` keep their signatures and use the defaults. `read_idle` bounds a whole frame, so a peer dripping bytes times out; on a server connection the first request is bounded by `handshake`. New `Error::Timeout { what }` and `From<std::io::Error>`. Slow-loris and never-terminated requests no longer hold a connection or grow memory: no awaited IO is unbounded. A deployment that keeps long-idle clients connected must raise `read_idle`.

Header blocks are capped at 64 KiB while no `Content-Length` is known, and request bodies at 2 MiB (was a single 2 MiB buffer cap); the client core rejects an unterminated header over 64 KiB, and its inbound buffer (and the server's request buffer) is capped (1 MiB / 2 MiB, #1088). The 64 KiB cap uses an incremental `memchr` search resumed from the last scanned offset, so `rtsp_types::Message::parse` runs once per message instead of once per read and an unterminated header can no longer force quadratic re-parsing.

### 4. Other behaviour changes

- **SETUP reply carries one `Transport` spec.** The reply's `Transport` header carries only the chosen (first) spec rather than every offered one (§12.39); `negotiated_transport()` and `ServerEvent::SessionSetup` hold that single spec.
- **Unparseable `Transport` on SETUP is a protocol error, not a dropped connection.** It is answered `461 Unsupported Transport` instead of returning `Err`.
- `ServerEvent::MediaData` (additive; the enum is `#[non_exhaustive]`): an interleaved `$`-framed block (RFC 2326 §10.12) received by a server is surfaced instead of an error. A TCP-interleaved PLAY client sending RTCP receiver reports (ffmpeg, VLC and GStreamer all do) used to have its connection dropped at the first one (#1088).

## New capabilities

- `AsyncRtspClient::announce` (with an SDP body), `record` and `send_interleaved` (a client-side `$`-framed media send, bounded by `RtspTimeouts::write`), built on `ClientSession::announce`/`record`. `send_interleaved` takes the payload by borrow. These support an RTSP pusher.
- `AsyncRtspClient::poll_keepalive` drives the `GET_PARAMETER` liveness ping for a send-only pusher and is called automatically by `send_interleaved`, which also drains buffered server-to-client bytes. Before this, a client that only sent interleaved media emitted no keepalive and its session expired (RFC 2326 §12.37). A `Timeout` or cancelled interleaved send leaves a partial frame and the connection is dead: reconnect, do not retry.
- `InterleavedFrame::slice_to_bytes(channel, &[u8])`: wire bytes from a borrowed payload, byte-identical to `new` plus `to_bytes`.
- `ClientSession::{mark_activity, poll_timeout, handle_timeout, has_buffered_input, peek_next_cseq}`: a keepalive deadline (half the `Session` timeout, default 60 s) driven by the adapter; only requests written count as activity. `recv_interleaved` now returns an error, not a clean end, when the peer closes mid-frame.

## Fixes

- **Authenticated retry lost its body (#1065).** `ClientSession`'s 401 auth retry replays the original request's body and every non-hop-by-hop header (`Content-Type` included). An authenticated `ANNOUNCE` previously lost its SDP on retry, so a push to any Digest- or Basic-challenging server always failed after the first 401.
- **Digest authenticator rebuilt from every 401 (#1088).** A server that rotated its nonce per session or time window without `stale=true` made the client sign the retry with the stale nonce and give up. `stale` detection is case-insensitive, tolerates a quoted value, and no longer misses `stale=true` when it is the first parameter or the realm contains a comma.
- `ClientSession::handle_data` no longer aborts the whole call, discarding already decoded events, on one response with an unknown or duplicate `CSeq`; it is skipped (#1088).
- `AsyncRtspClient::exchange` matches the response against the `CSeq` it just sent, not whichever `Response` was last in a decoded batch; its `pending_media` buffer is capped (oldest frame dropped) (#1088).
- A `454 Session Not Found` answering our `GET_PARAMETER` keepalive surfaces as `Error::SessionNotFound { method }` from the next `send_interleaved` instead of being discarded; the drain consumes at most `MAX_DRAIN_ITEMS` (64) per call so a fast peer cannot starve the pusher's own send.
- `AsyncRtspServer::next_request` is cancel-safe on the response write, and parses only once a blank line has arrived instead of re-parsing an unterminated header on every 8 KiB read.

## Dependencies

Verified from the `Cargo.toml` diff against `rtsp-runtime-v0.6.0`:

```toml
broadcast-auth = { version = "0.3" -> "0.4" }
sdp-types = "0.1"   # removed from [dependencies]; dev-dependency is now "0.2"
memchr = "2"        # new, always on
http-auth = { version = "0.1", default-features = false }   # new, always on
# tokio feature (all optional)
getrandom = { version = "0.2" }   # OS session ids
tokio-util = { version = "0.7", default-features = false, features = ["codec"] }
futures-util = { version = "0.3", default-features = false, features = ["sink", "std"] }
bytes = "1"
tokio = ["dep:tokio"] -> ["dep:tokio", "dep:getrandom", "dep:tokio-util", "dep:futures-util", "dep:bytes"]
```

The crate description no longer names `sdp-types`. MSRV 1.95.0.

---

Published from tag `rtsp-runtime-v0.7.0`.
