# srt-runtime

[![crates.io](https://img.shields.io/crates/v/srt-runtime.svg)](https://crates.io/crates/srt-runtime)
[![docs.rs](https://img.shields.io/docsrs/srt-runtime)](https://docs.rs/srt-runtime)

SRT (Secure Reliable Transport): packet codecs plus sans-IO handshake, ARQ, delivery-timing and congestion-control engines, with optional payload encryption and an optional `tokio` UDP adapter. Grounded in `draft-sharabayko-srt-01`.

```toml
[dependencies]
srt-runtime = "0.5"
# srt-runtime = { version = "0.5", features = ["tokio", "crypto"] }
```

## What it does / what it does not

**Does:** parse and serialize every SRT packet type byte-exactly; run the HSv5 Caller/Listener and Rendezvous handshakes as state machines; run the ARQ reliability engine (loss detection, NAK, ACK/ACKACK, RTT), the TSBPD delivery scheduler (timestamp-based ordered release and too-late drop), LiveCC pacing and FileCC window congestion control; and, behind features, do the AES-CTR payload crypto with key wrap and KM refresh, and drive all of it over real UDP sockets. The core is `no_std` + `alloc` and reads no clock: you call `tick()` and pass time in.

**Does not:** wrap an MPEG-TS stream or choose a payload for you, implement the legacy Version-4 Rendezvous path (permanently out of scope), or implement CUBIC/BBR file-transfer congestion control. The sans-IO congestion controllers are not wired into `arq::Sender` (it has no CC hook), and the tokio adapter **refuses encrypted connections** (a `HandshakeConfig` with `crypto` set is rejected, and encrypted data packets are never delivered) because its data path does not yet encrypt or decrypt. Encryption is negotiated and tested at the sans-IO level only.

Typical users: an SRT ingest or contribution endpoint (`multimux` builds its SRT input and push on this crate), test peers, and tooling that needs to inspect SRT packets.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| SRT protocol, IETF Internet-Draft `draft-sharabayko-srt` | -01 (vendored `specs/ietf_draft_sharabayko_srt_01.txt`; rules in `specs/rules/srt-*.md`) | §3 packet structure (data, every control type, handshake extensions, Key Material, Stream ID, Group Membership); §4.3.1 Caller/Listener; §4.3.2 Rendezvous (HSv5); §4.5-§4.7 TSBPD and too-late drop; §4.8, §4.10 ARQ and RTT; §5.1 LiveCC; §5.2 FileCC; §6 encryption | Partial | A draft, not a ratified RFC. Omissions: V4 rendezvous, CUBIC/BBR, CC wired into the sender, encryption in the tokio data path |
| RFC 3394 AES Key Wrap | RFC 3394 (not vendored, unverified here) | Wrapping the SEK | Full, behind `crypto` | |
| RFC 8018 PBKDF2 (HMAC-SHA1) | not vendored, unverified here | KEK derivation from a passphrase | Full, behind `crypto` | The draft specifies PBKDF2 over HMAC-SHA1 |
| libsrt 1.5.5 (reference implementation) | Used as an interop oracle, not a standard | Handshake, key exchange, loss recovery, rendezvous, MPEG-TS media | Verified | `tests/libsrt_*.rs`, see below |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`; the packet codecs and sans-IO engines are fully `no_std`. |
| `serde` | no | `Serialize` on the public wire types. |
| `crypto` | no | `crypto` (AES-CTR, RFC 3394 key wrap, PBKDF2/HMAC-SHA1), `handshake_sm::CryptoConfig` key exchange in the handshake, `km_refresh::KmRefreshDriver` (§6.1.6). Std-only dependencies; secrets are wiped on drop and compared in constant time. |
| `tokio` | no | `io::SrtSocket` and `io::SrtListener` over UDP (implies `std`): each connection sleeps until its own next deadline, datagrams are `bytes::Bytes`, `io::IoConfig` sets timeouts and `max_datagram`, rendezvous via `SrtSocket::rendezvous`. Randomised ISN and socket id. |

## Quick start

Parse a real `libsrt` KEEPALIVE (captured from `srt-live-transmit` 1.5.5) and write it back byte-for-byte:

```rust
use srt_runtime::packet::{ControlPacket, SrtPacket};

let bytes = std::fs::read("tests/fixtures/libsrt_keepalive.bin").unwrap();
let packet = SrtPacket::parse(&bytes).unwrap();
assert!(matches!(packet, SrtPacket::Control(ControlPacket::KeepAlive(_))));

let mut out = vec![0u8; packet.serialized_len()];
packet.serialize_into(&mut out).unwrap();
assert_eq!(out, bytes);
```

Run a Caller/Listener handshake in memory (no sockets; the bytes one side emits are parsed and fed to the other):

```rust
use srt_runtime::HandshakeConfig;
use srt_runtime::caller::{CallerHandshake, CallerHandshakeState};
use srt_runtime::handshake_sm::HandshakeOutput;
use srt_runtime::listener::{ListenerHandshake, ListenerHandshakeState};
use srt_runtime::packet::ControlPacket;

fn first_send(outputs: Vec<HandshakeOutput>) -> Option<Vec<u8>> {
    outputs.into_iter().find_map(|o| match o {
        HandshakeOutput::Send(bytes) => Some(bytes),
        _ => None,
    })
}

let mut caller = CallerHandshake::new(
    0x1111_1111,
    HandshakeConfig { latency_ms: 180, stream_id: Some("live/camera-1".into()), ..HandshakeConfig::default() },
);
// A real listener derives the SYN cookie from the peer address and a secret.
let mut listener = ListenerHandshake::new(
    0x2222_2222,
    0xC0FF_EE42,
    HandshakeConfig { latency_ms: 250, ..HandshakeConfig::default() },
);

let mut to_listener = caller.start().unwrap();
for _ in 0..10 {
    let pkt = ControlPacket::parse(&to_listener).unwrap();
    let Some(reply) = first_send(listener.feed(&pkt).unwrap()) else { break };
    let pkt = ControlPacket::parse(&reply).unwrap();
    match first_send(caller.feed(&pkt).unwrap()) {
        Some(next) => to_listener = next,
        None => break,
    }
}
assert_eq!(caller.state(), CallerHandshakeState::Connected);
assert_eq!(listener.state(), ListenerHandshakeState::Connected);
// Both sides agree on the greater of the two latencies.
assert_eq!(caller.negotiated().unwrap().latency_ms, 250);
```

Runnable programs: `handshake_loopback` and `arq_loss_recovery` (`cargo run -p srt-runtime --example <name>`).

## What is implemented

- **`packet`:** `SrtPacket::{Data, Control}`; `DataPacket` (sequence, message number, position, order, retransmit and encryption flags); every `ControlPacket` (Handshake with Handshake Extension, Key Material, Stream ID and Group Membership extensions; KeepAlive; ACK full, small and light; NAK with loss-list coding; Congestion Warning; Shutdown; ACKACK; Message Drop Request; Peer Error). Reserved and fixed-value fields are validated on parse and rebuilt on serialize.
- **Handshakes:** `CallerHandshake`, `ListenerHandshake`, `RendezvousHandshake` (cookie contest, Initiator or Responder at runtime), shared `HandshakeConfig`, `NegotiatedParams`, `HandshakeOutput`, `RejectionReason`.
- **`arq`:** `Sender`, `Receiver`, wrap-safe `seq` arithmetic (over `broadcast_common::seq::SeqSpace`), `rtt::RttEstimator`.
- **`tsbpd`:** `TsbpdScheduler`. **`livecc`:** `LiveCC` and `MaxBwConfig` (MAXBW_SET, INPUTBW_SET, INPUTBW_ESTIMATED). **`filecc`:** `FileCc` (Slow Start then Congestion Avoidance).
- **`crypto`, `km_refresh`** (feature `crypto`) and **`io`** (feature `tokio`) as in the features table. The tokio adapter is verified against a real libsrt peer including rendezvous.

## Design notes

Sans-IO and clock-free (`tick()` and caller-supplied time), `#![forbid(unsafe_code)]`, `no_std` core, errors as structured `Error` (never a panic on hostile input; a `no_panic` test sweeps the parsers), `#[non_exhaustive]` enums, labelled spec enums. The tokio adapter bounds every IO and cancels its background tasks when the listener and all accepted connections drop.

## Testing against libsrt

The `tests/libsrt_*.rs` files use a real `srt-live-transmit` as an independent oracle for handshake, key exchange, loss recovery, rendezvous, shutdown and MPEG-TS media checked with `ffprobe`. A missing tool makes a test skip loudly (a no-op pass, not coverage). Set `SRT_REQUIRE_LIBSRT=1` to make every skip a failure, for example in CI:

```sh
SRT_REQUIRE_LIBSRT=1 cargo test -p srt-runtime --all-features --locked
```

## Related crates

Depends on `broadcast-common`. `multimux` uses it for SRT input and push. `rist-runtime` is the architectural sibling (the same ARQ-over-UDP idea for RIST). `media-plane` carries the received bytes onward.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
