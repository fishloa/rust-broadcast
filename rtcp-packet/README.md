# rtcp-packet

[![Crates.io](https://img.shields.io/crates/v/rtcp-packet.svg)](https://crates.io/crates/rtcp-packet)
[![docs.rs](https://img.shields.io/docsrs/rtcp-packet)](https://docs.rs/rtcp-packet)

RTCP control packets (RFC 3550 §6): SR, RR, SDES, BYE, APP and the §6.1 compound packet. Spec-complete parse and serialize, `no_std`.

```toml
[dependencies]
rtcp-packet = "0.5"
```

## What it does / what it does not

**Does:** parse and build the five core RTCP packet types as typed structs, plus a `CompoundPacket` that holds a whole UDP datagram of them and round-trips it byte-for-byte. Any other packet type (transport and payload-specific feedback such as NACK, PLI, REMB, extended reports) is kept as `RtcpPacket::Unknown` with its opaque body, `P` bit and padding preserved, so a real browser compound is never rejected or damaged.

**Does not:** decode feedback packets (RFC 4585 Generic NACK is decoded in `rist-runtime`; PLI, REMB and XR are carried opaque), schedule reports (RFC 3550 §6.2/§6.3 timing), compute jitter or loss statistics, or touch sockets. It carries no media and is a pure control-channel codec.

Typical users: RTSP/RTP clients and servers, WebRTC engines, RIST, and monitoring tools.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF RFC 3550 "RTP" | RFC 3550 (vendored `specs/rfc3550_rtp.txt`) | §6.1 compound packets, §6.4.1 SR, §6.4.2 RR, §6.4.1 report block (24-bit signed `cumulative_lost`), §6.5 SDES (CNAME, NAME, EMAIL, PHONE, LOC, TOOL, NOTE, PRIV), §6.6 BYE, §6.7 APP, §6.4.1 profile-specific extensions | Full | SDES PRIV's internal `prefix`/`value` split is not separately typed (the text is kept flat and round-trips). See `docs/rtcp.md` |
| IETF RFC 5506 "Reduced-Size RTCP" | RFC 5506 (not vendored) | A compound that starts with a non-SR/RR packet is accepted | Partial | Only the leading-packet relaxation, no feedback semantics |
| IETF RFC 4585 / RFC 5104 / RFC 3611 / REMB draft | n/a | PT 205, 206, 207 and XR | Typed-opaque | Preserved as `RtcpPacket::Unknown` |

Verified against a real Chromium `RTCPeerConnection` compound (RR + PLI + REMB, 68 bytes, `tests/fixtures/chrome_rr_pli_remb.bin`) plus spec-table vectors.

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`. |
| `serde` | no | `serde::Serialize` on the public types. |

## Quick start

Parse the real Chromium compound, inspect each packet, and write it back unchanged:

```rust
use broadcast_common::{Parse, Serialize};
use rtcp_packet::{CompoundPacket, RtcpPacket};

let wire = std::fs::read("tests/fixtures/chrome_rr_pli_remb.bin").unwrap();
let compound = CompoundPacket::parse(&wire).unwrap();
assert_eq!(compound.packets.len(), 3);

match &compound.packets[0] {
    RtcpPacket::ReceiverReport(rr) => {
        assert_eq!(rr.report_blocks[0].ext_highest_seq, 5146);
    }
    other => panic!("expected RR first, got {}", other.name()),
}
// PLI and REMB are feedback packets: kept opaque, never dropped.
assert!(matches!(compound.packets[1], RtcpPacket::Unknown { .. }));

assert_eq!(compound.try_to_bytes().unwrap(), wire);
```

Runnable programs: `cargo run -p rtcp-packet --example build_sender_report` and `--example parse_compound_packet`.

## What is implemented

- `SenderReport` / `ReceiverReport` (constructors `new` and `with_profile_extensions`) and `ReportBlock`.
- `SourceDescription`, `SdesChunk`, `SdesItem`, `SdesItemType` with 32-bit chunk padding.
- `Bye` (SSRC/CSRC list and optional reason) and `App` (subtype, SSRC, 4-byte ASCII name, data).
- `RtcpPacket` and `RtcpPacketType` for PT dispatch (`from_pt`, `pt`, `name`), and `CompoundPacket`.
- Padding: typed packets validate and strip padding on parse and write `P = 0`; `Unknown` keeps it verbatim.
- Bounds are enforced on write: a `cumulative_lost` outside the 24-bit signed range is `Error::InvalidValue` rather than silently masked; text fields are UTF-8 validated before allocation.

## Design notes

Symmetric `Parse`/`Serialize` with byte-exact compound round-trip, `#[non_exhaustive]` enums and `Error`, `label_coverage` guard for `name()` plus `Display`, no raw-byte public API for the typed packets, `no_std` + `alloc`.

## Related crates

Depends only on `broadcast-common`. `rtp-packet` is the data-plane companion. Built on by `rist-runtime` (Generic/Range NACK, RTT echo, compound builders), `webrtc-runtime` and `transmux` (RTP/RTCP). It was extracted from `transmux::rtcp`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
