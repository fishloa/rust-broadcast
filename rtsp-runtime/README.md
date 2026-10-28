# rtsp-runtime

[![crates.io](https://img.shields.io/crates/v/rtsp-runtime.svg)](https://crates.io/crates/rtsp-runtime)
[![docs.rs](https://img.shields.io/docsrs/rtsp-runtime)](https://docs.rs/rtsp-runtime)

Sans-IO RTSP 1.0 (RFC 2326) session engine: a driveable client and a server state machine, `Transport` negotiation, interleaved RTP/RTCP framing and authenticated requests. An optional `tokio` adapter (and `tls` for `rtsps://`) puts it on real sockets.

```toml
[dependencies]
rtsp-runtime = "0.8"                                       # sans-IO core
rtsp-runtime = { version = "0.8", features = ["tokio"] }   # + real sockets
rtsp-runtime = { version = "0.8", features = ["tls"] }     # + rtsps:// (TLS)
```

## What it does / what it does not

**Does:** run the RFC 2326 Appendix A session state machines (`Init`, `Ready`, `Playing`, `Recording`) on both sides; build outbound requests (OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN, ANNOUNCE, RECORD, GET_PARAMETER) with an incrementing `CSeq` and match responses back to them; parse and serialize the `Transport` (§12.39) and `Session` (§12.37) headers; frame and de-frame `$`-channel interleaved RTP/RTCP (§10.12); answer `401` challenges with Basic, Digest or Bearer credentials, including `stale=true` nonce refresh; and manage the keepalive timeout. The server rejects a method that is illegal in the current state with `455`, allocates a `Session` id on SETUP (from a caller-supplied CSPRNG), and echoes the first offered transport.

**Does not:** open sockets in the core (feed bytes in, get bytes and events out), parse RTP or RTCP payloads (use `rtp-packet` and `rtcp-packet`), parse SDP (use `sdp-types`), generate DESCRIBE SDP or challenge clients on the server side (the server session has no authentication; put a `broadcast-auth` `Verifier` in your own layer), or speak RTSP 2.0 (RFC 7826 is vendored but not implemented). The client has no SET_PARAMETER builder, and server-initiated REDIRECT is not modelled.

Typical users: IP-camera ingest (`multimux`'s RTSP input), RTSP push to a media server, test servers, and tools that need to drive a camera session deterministically.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF RFC 2326 "Real Time Streaming Protocol" | RFC 2326 (vendored `specs/rfc2326_rtsp.txt`, transcribed in `rtsp-runtime/docs/rfc2326.md`) | Appendix A state machines, §10 methods listed above, §10.12 interleaved framing, §12.37 `Session`, §12.39 `Transport`, §14 authentication, CSeq (§12.17) | Partial | See the methods table below |
| IETF RFC 7826 "RTSP 2.0" | RFC 7826 (vendored, not implemented) | n/a | Not implemented | |
| IETF RFC 7617 / RFC 7616 / RFC 6750 (Basic, Digest, Bearer) | not vendored, unverified here | Client side via `broadcast-auth` | Full | Server side has no authentication |
| IETF RFC 8866 (SDP) | RFC 8866 (vendored) | Bodies carried opaque | Typed-opaque | Parse with `sdp-types` |
| IETF RFC 3550 (RTP/RTCP) | RFC 3550 | Interleaved payload is carried as bytes | Typed-opaque | Decode with `rtp-packet` / `rtcp-packet` |
| `rtsps://` (RTSP over TLS, default port 322) | n/a | Client `connect_tls`, server `accept_tls` | Full, behind `tls` | `webpki-roots` trust by default or a custom `rustls::ClientConfig` |

| Method | Client builds | Server session handles | State effect |
|---|---|---|---|
| OPTIONS, DESCRIBE, GET_PARAMETER, ANNOUNCE | yes | accepted, empty 200 | none |
| SET_PARAMETER | no builder | accepted as state-neutral, empty 200 | none |
| SETUP | yes | allocates `Session`, echoes transport | Init to Ready |
| PLAY, PAUSE, RECORD, TEARDOWN | yes | accepted, requires `Session` | Ready/Playing/Recording per Appendix A |
| REDIRECT | no | not modelled (a client resets to `Init` on a `3xx`) | n/a |

## Features

| Feature | Default | Description |
|---|---|---|
| `serde` | no | `Serialize` on the public wire and event types. |
| `tokio` | no | `AsyncRtspClient` / `AsyncRtspServer` (`tokio_util` `Framed` adapters; 64 KiB header and 2 MiB message caps; `RtspTimeouts`; keepalive at half the session timeout). |
| `tls` | no | Implies `tokio`; adds `rtsps://` via `tokio-rustls`. |

The core is `std` and uses no async runtime. The crate has `#![forbid(unsafe_code)]`.

## Quick start

A client talking to a server entirely in memory (no socket):

```rust
use rtsp_runtime::{
    ClientEvent, ClientSession, InterleavedFrame, ServerSession, SessionState, Transport,
    TransportSpec,
};

let url = "rtsp://127.0.0.1/stream";
let track = "rtsp://127.0.0.1/stream/trackID=0";

// A fixed id is fine here; a real server must draw ids from a CSPRNG.
let mut server = ServerSession::new(|| 0x0123_4567_89ab_cdef);
let mut client = ClientSession::new();

let req = client.options(url).unwrap();
let (resp, _) = server.handle_request(&req).unwrap();
client.handle_data_strict(&resp).unwrap();

let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
let req = client.setup(track, &transport).unwrap();
let (resp, _) = server.handle_request(&req).unwrap();
client.handle_data_strict(&resp).unwrap();
assert_eq!(client.state(), SessionState::Ready);
assert!(client.session_id().is_some());

let req = client.play(url).unwrap();
let (resp, _) = server.handle_request(&req).unwrap();
client.handle_data_strict(&resp).unwrap();
assert_eq!(client.state(), SessionState::Playing);

// While playing, RTP is tunnelled on interleaved channel 0 over the same connection.
let rtp = [0x80, 0x60, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1, 0xde, 0xad];
let wire = InterleavedFrame::slice_to_bytes(0, &rtp).unwrap();
let events = client.handle_data_strict(&wire).unwrap();
assert!(matches!(&events[0], ClientEvent::MediaData { channel: 0, data } if data == &rtp));

let req = client.teardown(url).unwrap();
let (resp, _) = server.handle_request(&req).unwrap();
client.handle_data_strict(&resp).unwrap();
assert_eq!(client.state(), SessionState::Init);
```

Runnable programs: `client_server_loopback` and `transport_and_framing` (`cargo run -p rtsp-runtime --example <name>`).

## What is implemented

- `ClientSession`: request builders, `handle_data` (returns `HandleOutcome { events, error }` so frames decoded before a terminal error are never lost) and `handle_data_strict`; `with_credentials`; `AuthRetry` events for `401`; `poll_timeout`/`handle_timeout` keepalive.
- `ServerSession`: `handle_request` returning response bytes plus `ServerEvent`s (`RequestAccepted`, `MethodNotValid`, `SessionSetup`, `MediaData`), `with_session_timeout`.
- `Transport`, `TransportSpec`, `LowerTransport`, `Delivery`, `TransportMode`: UDP unicast and multicast, TCP interleaved. `SessionHeader`: `Session` id and `timeout`. Both are parsed by this crate over one RFC 2326 §15.1 lexer, because `rtsp-types` 0.1 is exact-case and drops case-variant parameters (a documented, owner-approved exception to the no-hand-roll rule; probe table in `docs/transport-header.md`).
- `InterleavedFrame` and a streaming demultiplexer (complete frames plus a partial tail).
- `Credentials` and `Authenticator`, re-exported from `broadcast-auth`.
- `io` (feature `tokio`): `AsyncRtspClient`, `AsyncRtspServer`, `RtspTimeouts`, `RTSP_DEFAULT_PORT` (554), `RTSPS_DEFAULT_PORT` (322).

## Design notes

Sans-IO: the core never reads a clock or a socket (the caller passes `Instant` for timeouts and supplies the session-id source). `Error` is `#[non_exhaustive]`. The `Session` id comparison is constant time. Message framing is bounded (header and message size caps). Tests include golden wire transcripts and real-loopback integration tests (OPTIONS to TEARDOWN, fragmented interleaved media, Digest, a TLS handshake with a self-signed certificate). A lexical `no_handroll_guard` test enforces the allowlist.

## Related crates

Builds on `rtsp-types` and `sdp-types` (codecs) and `broadcast-auth`. Pair with `rtp-packet` and `rtcp-packet` for the interleaved payloads. `multimux` uses it for RTSP input and push, and `transmux` for RTP depacketising.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
