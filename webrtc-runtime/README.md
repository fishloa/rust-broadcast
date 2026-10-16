# webrtc-runtime

[![crates.io](https://img.shields.io/crates/v/webrtc-runtime.svg)](https://crates.io/crates/webrtc-runtime)
[![docs.rs](https://img.shields.io/docsrs/webrtc-runtime)](https://docs.rs/webrtc-runtime)

Sans-IO WHIP (RFC 9725) and WHEP (draft-ietf-wish-whep) HTTP signalling engines for WebRTC session setup, plus an optional ICE and DTLS-SRTP media transport.

## What it does / what it does not

**Does:** provide caller-driven state machines for both ends of both protocols. WHIP is ingest (`WhipClient` is the encoder, `WhipSession` is the server's session resource); WHEP is egress (`WhepPlayer` is the viewer, `WhepSession` is the server's session resource). Each client produces an `HttpRequest` descriptor (method, URL, `http::HeaderMap`, body) for you to send with any HTTP stack, and consumes the `HttpResponse` you feed back. They cover the SDP offer/answer exchange, Trickle ICE (PATCH), ICE restart, teardown (DELETE), the WHEP counter-offer (406) and no-publisher (409) paths, `ETag`/`If-Match` handling, Bearer authentication, and ICE-server discovery through `Link` headers. With the `media` feature, `media::MediaTransport` adds the ICE agent, the DTLS handshake with RFC 5764 SRTP key export, and SRTP/SRTCP encryption and decryption into this workspace's own `rtp-packet` and `rtcp-packet` types.

**Does not:** do HTTP, sockets, TLS or an async runtime (the caller drives them; there is no tokio adapter), generate or interpret SDP (the caller extracts `a=ice-ufrag`, `a=ice-pwd`, `a=fingerprint` and `a=candidate` and passes them in), encode or decode media, use TURN relay candidates in `media` (host and server-reflexive only), or implement simulcast, SVC or data channels.

Security defaults: a Bearer token is sent only to the origin (scheme, host, port) of the configured endpoint, so a session URL on another origin gets no `Authorization`; `media` requires the remote DTLS certificate fingerprint and verifies it three ways (constant-time compare); secrets are redacted from `Debug`.

Typical users: `multimux`'s `whip` and `whep` features, a WebRTC test client, an SFU front end.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF RFC 9725 WHIP | RFC 9725 (vendored `specs/rfc9725_whip.txt`; `docs/whip-rfc9725.md`) | §4 offer POST (201), Trickle ICE and ICE restart PATCH (`application/trickle-ice-sdpfrag`, `If-Match`; a missing `If-Match` is `428`), DELETE, `Link` ICE servers, Bearer | Full for signalling | The server resource enforces content type and entity tags |
| draft-ietf-wish-whep | -04 (`docs/whep-draft-04.md`; not in the vendored index) | Offer POST, counter-offer (406 then PATCH `application/sdp`), 409 no publisher, PATCH, DELETE | Full for signalling | A draft; later revisions are not tracked |
| IETF RFC 8840 / RFC 8839 (Trickle ICE SDP fragments) | not vendored, unverified here | Fragment bytes carried opaque | Typed-opaque | |
| IETF RFC 8445 ICE, RFC 8489 STUN | RFC 8445, RFC 8489 (transcribed in `docs/`) | `media`: host and server-reflexive candidates, connectivity checks, `restart_ice` | Partial | No TURN relay. Verified with RFC 5769 STUN vectors |
| IETF RFC 5764 DTLS-SRTP, RFC 8122 fingerprints | RFC 5764 (transcribed) | `media`: DTLS handshake, SRTP key export, fingerprint pinning, rekey (`needs_rekey`, `rekey`) | Full for a single flow | Peer must match the SDP `a=fingerprint` |
| IETF RFC 3711 SRTP | RFC 3711 (transcribed) | `media`: SRTP/SRTCP protect and unprotect, 2^31 packet key lifetime | Full | Verified with the RFC 3711 Appendix B vectors |
| IETF RFC 5761 RTP/RTCP mux | RFC 5761 (transcribed) | `media`: demultiplexing STUN, DTLS, RTP and RTCP on one port | Full | |
| IETF RFC 6750 Bearer | not vendored, unverified here | Bearer token on requests | Full | Origin-scoped |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | A feature name kept for `default-features = false` consumers. The crate is `std` regardless (it speaks `http` and `headers` types). |
| `serde` | no | `Serialize`/`Deserialize` on `IceServer`. |
| `media` | no | `media::MediaTransport`: ICE, DTLS-SRTP, SRTP over the `rtc-*` crates. Implies `std`. Optional on dependency weight only. |
| `test-support` | no | Hidden fault-injection and loopback hooks for downstream test suites. Not public API. |

## Quick start

A WHIP encoder and a WHIP server resource talking to each other in memory (no HTTP stack):

```rust
use webrtc_runtime::whip::client::{Event as ClientEvent, WhipClient};
use webrtc_runtime::whip::server::{Event as ServerEvent, WhipSession};

let mut client = WhipClient::new("https://whip.example.com/pub".into(), Some("token".into()));
let mut session = WhipSession::new("https://whip.example.com/session/abc123".into());

// 1. The client builds the POST; the server resource reads the offer.
let post = client.offer(b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n".to_vec()).unwrap();
assert_eq!(post.method, http::Method::POST);
let ServerEvent::SdpOffer(offer) = session.on_post(post.body).unwrap() else { panic!() };
assert!(offer.starts_with(b"v=0"));

// 2. The server answers with 201, an ETag and the session Location.
let created = session.accept(b"v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n".to_vec(), "v1".into());
let event = client.on_response(created).unwrap();
assert!(matches!(event, Some(ClientEvent::SdpAnswer(_))));

// 3. Trickle ICE: the client PATCHes, the server checks If-Match and acknowledges.
let patch = client.flush_candidates(b"a=ice-ufrag:u\r\na=ice-pwd:p\r\n".to_vec()).unwrap();
session.on_patch(patch.body, &patch.headers).unwrap();
client.on_response(session.ack_trickle()).unwrap();

// 4. Teardown.
let delete = client.terminate().unwrap();
assert_eq!(delete.method, http::Method::DELETE);
session.on_delete().unwrap();
```

ICE servers advertised in `Link` headers round-trip through `ice::parse_ice_server_links` and `ice::format_ice_server_links`:

```rust
use webrtc_runtime::ice::{format_ice_server_links, parse_ice_server_links};

let servers = parse_ice_server_links(
    r#"<stun:stun.example.com>; rel="ice-server", <turn:turn.example.com>; rel="ice-server"; username="u"; credential="c""#,
);
assert_eq!(servers.len(), 2);
let header = format_ice_server_links(&servers);
assert!(header.contains("turn.example.com"));
```

Runnable programs: `whip_lifecycle`, `ice_server_links` and `whip_media_smoke` (the last needs `--features media`), via `cargo run -p webrtc-runtime --example <name>`.

## What is implemented

- **`whip`:** `client::WhipClient` (`offer`, `flush_candidates`, `ice_restart`, `terminate`, `on_response`) and `server::WhipSession` (`on_post`, `accept`, `on_patch`, `ack_trickle`, `ack_restart`, `on_delete`, `ack_delete`), each with a typed `State` and `Event`.
- **`whep`:** `player::WhepPlayer` and `server::WhepSession`, including the counter-offer flow and the `CounterOffered` state.
- **`ice`:** `IceServer`, `parse_ice_server_links`, `format_ice_server_links` (RFC 9725 §4.4, the `Link` header is a documented hand-rolled exception to the no-hand-roll guard because no crate parses RFC 8288 links with these parameters).
- **`media`** (feature `media`): `MediaTransport` with `handle_datagram`, `poll_transmit`, `handle_timeout`, `poll_timeout`, `add_remote_candidate`, `restart_ice` (in place, keeps DTLS and SRTP), `encrypt_rtp`, `encrypt_rtcp`, `needs_rekey`, `rekey`, plus `MediaTransportConfig` and `MediaEvent`.
- **`Error`** is `#[non_exhaustive]` and distinguishes wrong-state calls, missing or wrong preconditions (`PreconditionRequired`), bad content types and media errors.

## Design notes

Sans-IO and clock-free (`media` takes `Instant` from the caller and schedules from `poll_timeout`), typed `http` and `headers` accessors rather than string headers, `#[non_exhaustive]` enums and errors, golden HTTP transcripts and a no-hand-roll guard. The `media` transport is exercised by RFC test vectors and by loopback tests, not only mocks.

## Related crates

`rtp-packet` and `rtcp-packet` (the `media` feature types its output with them); `broadcast-common`. `multimux` builds its `whip` input and `whep` output on this crate.

## MSRV and licence

MSRV 1.95.0, for every feature. Licensed under MIT OR Apache-2.0.
