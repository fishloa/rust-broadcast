# rist-runtime

[![crates.io](https://img.shields.io/crates/v/rist-runtime.svg)](https://crates.io/crates/rist-runtime)
[![docs.rs](https://docs.rs/rist-runtime/badge.svg)](https://docs.rs/rist-runtime)

RIST Simple Profile (VSF TR-06-1:2020) RTCP message codecs plus a sans-IO NACK-based ARQ engine.

## What it does / what it does not

**Does:** parse and build the RTCP messages that make RIST retransmission work (Generic NACK, Range NACK, RTT Echo, and the sender/receiver compound packets), and provide a small, clock-free ARQ state machine (`arq::Receiver`, `arq::Sender`) that decides *when* to request a retransmission and *which* buffered packets to resend. Every wire type round-trips byte-for-byte.

**Does not:** open sockets, read a clock, build RTP media packets, do GRE tunnelling, encryption (TR-06-2 Main Profile), bonding, or null-packet deletion. The ARQ engine is driven by the caller, who passes `now: Duration` and moves bytes. Use it with `rtp-packet` for the media packets and your own UDP sockets (for example `tokio`).

Typical users: a RIST sender/receiver built on `tokio` UDP, a RIST proxy or monitor, or a test harness replaying captured RTCP.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| VSF TR-06-1 "RIST Protocol Specification, Simple Profile" | 2020 (vendored `vsf_tr-06-1_2020_rist_protocol_simple_profile`) | §5.2 RTCP support (SR/RR/SDES via `rtcp-packet`, §5.2.6 RTT Echo), §5.2.1 compound structure, §5.3.2.1 bitmask NACK, §5.3.2.2 range NACK, §5.3.3 retransmitted packets, §5.3.4 burst control | Full for the wire codecs; Partial for ARQ | The ARQ timing is mostly implementer's choice in the spec (§5.3.4) and follows librist; see the `arq` module docs for which numbers are spec and which are policy |
| VSF TR-06-1 §5.1 baseline protocol (port pairs, RTP framing) | 2020 | Port assignment, RTP header use | Not implemented | Caller owns sockets and RTP |
| VSF TR-06-1 §5.4 bonding | 2020 | Multi-path | Not implemented | |
| VSF TR-06-2 "RIST Main Profile" | 2024 (vendored) | Timing notes only, transcribed to `docs/tr-06-2-main-profile-timing.md` | Not implemented | Checked only to confirm it adds no ARQ timing formula |
| IETF RFC 4585 (RTCP feedback) | RFC 4585 | §6.1 common header, §6.2.1 Generic NACK (PT 205, FMT 1) | Full | |
| IETF RFC 3550 (RTP/RTCP) | RFC 3550 | §6.4.1 padding (P bit) validated and preserved | Full for padding | Packet types themselves come from `rtcp-packet` |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`. |

## Quick start

A loopback of the whole recovery path: a sender buffers four packets, the receiver misses two, asks for them with a Range NACK, and the sender answers.

```rust
use broadcast_common::{Parse, Serialize};
use core::time::Duration;
use rist_runtime::arq::{ArqConfig, Receiver, Sender};
use rist_runtime::RangeNack;

let ms = Duration::from_millis;

// Sender side: remember what was sent (packet count cap, not time).
let mut tx = Sender::new(64);
for seq in 100u16..=103 {
    tx.on_sent(seq, u32::from(seq) * 90, &[seq as u8; 4]);
}

// Receiver side: 101 and 102 never arrive.
let mut rx = Receiver::new(ArqConfig::appendix_b_defaults());
rx.feed(100, ms(0));
rx.feed(103, ms(1));

// After the 70 ms reorder section the gap is a confirmed loss.
let mut due = Vec::new();
let mut now = 80;
while due.is_empty() && now < 2_000 {
    due = rx.tick(ms(now)).due;
    now += 10;
}
assert_eq!(due.len(), 1);
assert_eq!((due[0].start, due[0].additional), (101, 1));

// Put the request on the wire and parse it back at the sender.
let nack = RangeNack { ssrc_media: 0x1234_5678, ranges: due, padding: vec![] };
let wire = nack.try_to_bytes().unwrap();
let received = RangeNack::parse(&wire).unwrap();

let resent = tx.on_range_nack(&received, ms(now));
let seqs: Vec<u16> = resent.iter().map(|r| r.seq).collect();
assert_eq!(seqs, [101, 102]);
```

## What is implemented

- **Wire codecs** (`GenericNack`, `RangeNack`, `RttEcho`): RFC 4585 §6.2.1 bitmask NACK (PT 205 FMT 1), RIST range NACK (APP, name `RIST`, subtype 0, at most 16 ranges), RTT Echo request/response (APP subtype 2/3). Padding (`P` bit) is validated and preserved.
- **Compound packets** (`RistSenderCompound`, `RistReceiverCompound`): the §5.2.1 structure (sender: SR or empty RR + SDES CNAME + optional RTT Echo; receiver: RR + SDES CNAME + optional Generic/Range NACKs + optional RTT Echo). Sub-packets the crate does not model are kept as `UnknownPacket` at their wire position so a compound round-trips byte-identically.
- **`arq::Receiver`**: sequence tracking with 16-bit wraparound, two-stage (reorder then reassembly) loss detection, RTT-driven retry scheduling, give-up on retry exhaustion or buffer age, and `ranges_to_fci`/`seqs_to_fci` helpers to choose the range or bitmask wire form.
- **`arq::Sender`**: bounded retransmission lookup buffer, per-sequence rate limit, NACK-amplification cap (a 65 536-packet range expansion limit).
- **`arq::rtt`**: `RttEstimator` (8-sample EWMA in integer ticks, after librist) and `rtt_sample` from an RTT Echo response.
- **Constants** for every PT/FMT/subtype are named; `name()` plus `Display` on the enums follows the workspace label convention.

## Design notes

- **Sans-IO and clock-free.** No sockets, no wall clock; the caller supplies `now: Duration` since an epoch it owns.
- **Round-trip invariant.** Every codec has a symmetric `Serialize`; unknown content is preserved, never dropped.
- **Errors.** `Error` is `#[non_exhaustive]` and carries context (`BufferTooShort`, `InvalidPaddingCount`, `PaddingBitSet`, ...).
- **Spec vs policy.** Where TR-06-1 leaves timing to the implementer, the code says so and cites librist (BSD-2-Clause, read directly) rather than presenting policy as spec.
- **`no_std` + `alloc`** without the `std` feature.

## Related crates

Depends on `broadcast-common` (traits) and `rtcp-packet` (generic RTCP). Pair with `rtp-packet` for media packets. `srt-runtime::arq` is the architectural sibling for SRT.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
