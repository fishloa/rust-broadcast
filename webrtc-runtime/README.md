# webrtc-runtime

Sans-IO **WHIP** ([RFC 9725](https://www.rfc-editor.org/rfc/rfc9725)) +
**WHEP** ([draft-ietf-wish-whep](https://www.ietf.org/archive/id/draft-ietf-wish-whep-04.html))
HTTP signalling engine for WebRTC session establishment, plus (feature
`media`) the ICE/DTLS-SRTP transport that carries the media those two
protocols negotiate.

Part of the [rust-broadcast](https://github.com/fishloa/rust-broadcast) workspace.

## What it does

Provides caller-driven state machines for both sides of two complementary protocols:

| Protocol | Direction | Client type | Server type |
|----------|-----------|-------------|-------------|
| **WHIP** | Ingest (encoder -> server) | `WhipClient` | `WhipSession` |
| **WHEP** | Egress (server -> viewer) | `WhepPlayer` | `WhepSession` |

Each state machine is **sans-IO**: it produces `HttpRequest` descriptors the
caller sends over whatever HTTP stack it prefers, and consumes `HttpResponse`
values fed back. No sockets, no async runtime, no TLS -- just signalling
logic and state transitions.

Feature `media` (see "MSRV" below) adds `webrtc_runtime::media::MediaTransport`:
the ICE agent, DTLS handshake + RFC 5764 SRTP key export, and SRTP decrypt
context that turns the SDP/candidates this signalling exchanges into an
actual UDP media path, decrypting inbound SRTP into RTP/RTCP typed by this
workspace's own `rtp-packet`/`rtcp-packet` crates. Still sans-IO: no socket
owned, no tokio runtime pulled into the core.

### Covered flows

- SDP offer/answer exchange (HTTP POST -> 201 Created)
- Trickle ICE candidate addition (HTTP PATCH, `application/trickle-ice-sdpfrag`)
- ICE restart (PATCH with `If-Match: "*"`)
- Session teardown (HTTP DELETE)
- WHEP counter-offer (406 Not Acceptable -> PATCH with `application/sdp`)
- WHEP no-publisher detection (409 Conflict)
- Bearer token authentication (RFC 6750)
- ICE server discovery via Link header parsing (RFC 9725 section 4.4)

## Usage

### WHIP client (encoder/ingester)

```rust
use webrtc_runtime::whip::client::{WhipClient, HttpResponse};

// Create a client pointing at the WHIP endpoint.
let mut client = WhipClient::new(
    "https://live.example.com/whip".into(),
    Some("my-bearer-token".into()),
);

// 1. Generate the SDP offer from your WebRTC stack, then:
let request = client.offer(sdp_offer_bytes).unwrap();
// ... send `request` via your HTTP client ...

// 2. Feed the 201 response back:
let event = client.on_response(HttpResponse {
    status: 201,
    content_type: Some("application/sdp".into()),
    location: Some("https://live.example.com/session/abc".into()),
    etag: Some("v1".into()),
    body: sdp_answer_bytes,
}).unwrap();
// event is Some(Event::SdpAnswer(...)) -- pass to your WebRTC stack.

// 3. Send trickle ICE candidates:
let request = client.flush_candidates(aggregated_fragment).unwrap();
// ... send PATCH ...

// 4. Tear down:
let request = client.terminate().unwrap();
// ... send DELETE ...
```

### WHEP player (viewer)

```rust
use webrtc_runtime::whep::player::{WhepPlayer, HttpResponse};

let mut player = WhepPlayer::new(
    "https://live.example.com/whep".into(),
    None,
);

let request = player.offer(sdp_offer_bytes).unwrap();
// ... send POST ...

// Direct accept (201) or counter-offer (406) -- the state machine
// handles both:
let event = player.on_response(http_response).unwrap();
// match event { Event::SdpAnswer(..) => ..., Event::CounterOffer(..) => ... }
```

### ICE server discovery

```rust
use webrtc_runtime::ice::{parse_ice_server_links, format_ice_server_links};

// Parse Link headers from WHIP/WHEP 201 response:
let servers = parse_ice_server_links(
    r#"<stun:stun.example.com>; rel="ice-server", <turn:turn.example.com>; rel="ice-server"; username="u"; credential="c""#
);
assert_eq!(servers.len(), 2);

// Serialize back for server responses:
let header = format_ice_server_links(&servers);
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enables `std` library support |
| `serde` | no      | `Serialize`/`Deserialize` on `IceServer` |
| `media` | no      | ICE + DTLS-SRTP transport (`webrtc_runtime::media`) — **requires rustc >= 1.88**, see "MSRV" below |

The core state machines build with `--no-default-features` (`no_std` + `alloc`).
No IO adapter exists for WHIP/WHEP signalling (see "What it does" above) —
there is no `tokio` feature for it; one was declared but never gated any
code, so it was removed rather than kept as dead weight (see CHANGELOG).

## MSRV

The crate's declared MSRV is **1.95.0**, the workspace MSRV, and every
feature builds on it.

This used to be a split: the crate declared 1.86 while `media` needed
**rustc >= 1.88** (its `rtc-dtls` dependency requires `rcgen ^0.14.8`, whose
own MSRV is 1.88), so `media` was excluded from the workspace
`--all-features` lanes and covered by a dedicated CI job. Raising the
workspace MSRV past 1.88 (issue #949) removed the gap, and with it the whole
containment apparatus.

`media` remains an optional feature — it pulls the ICE/DTLS-SRTP transport
and its dependency tree, which a consumer that only needs WHIP/WHEP
signalling has no reason to build. That is now a dependency-weight choice,
not a toolchain constraint.

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT License](../LICENSE-MIT) at your option.
