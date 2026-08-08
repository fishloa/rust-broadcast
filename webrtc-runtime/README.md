# webrtc-runtime

Sans-IO **WHIP** ([RFC 9725](https://www.rfc-editor.org/rfc/rfc9725)) +
**WHEP** ([draft-ietf-wish-whep](https://www.ietf.org/archive/id/draft-ietf-wish-whep-04.html))
HTTP signalling engine for WebRTC session establishment.

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

The core state machines build with `--no-default-features` (`no_std` + `alloc`).
No IO adapter exists (see "What it does" above) — there is no `tokio` feature;
one was declared but never gated any code, so it was removed rather than kept
as dead weight (see CHANGELOG).

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT License](../LICENSE-MIT) at your option.
