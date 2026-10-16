# st2022

[![crates.io](https://img.shields.io/crates/v/st2022.svg)](https://crates.io/crates/st2022)
[![docs.rs](https://docs.rs/st2022/badge.svg)](https://docs.rs/st2022)

SMPTE ST 2022-6 HBRMT (SDI-over-IP) RTP payload header parser and serializer.

## What it does / what it does not

**Does:** parse and build the High Bit Rate Media Transport (HBRMT) payload header that sits between the RTP fixed header and the media payload in an ST 2022-6 datagram: the video source id (`VSID`), frame count, timestamp reference, scrambling, FEC usage, clock frequency, the optional video source format (MAP/FRAME/FRATE/SAMPLE), the optional video timestamp, and the optional header extension. Every field is typed and the header round-trips byte-for-byte. It is checked against a real HBRMT capture.

**Does not:** parse the RTP fixed header (use `rtp-packet`), interpret the SDI media payload (video, embedded audio, VANC/HANC), do ST 2022-5 FEC, or do ST 2022-7 seamless protection switching. The crate does parse the `VSID`, which a ST 2022-7 merge needs to tell redundant-path copies apart, but the merge itself does not exist in this workspace yet (issue #752; `media-plane::byte_merge` is the expected home and documents the policy as deliberately absent).

Typical users: SDI-over-IP monitors, capture analysers, and test tooling for contribution links.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| SMPTE ST 2022-6 "Transport of High Bit Rate Media Signals over IP Networks (HBRMT)" | 2012 (vendored) | §6.4 High Bit Rate Media Payload Header (fixed 4 bytes, video source format, video timestamp, extension). §6.2/§6.3/§6.5 are transcribed in `docs/` but are RTP framing and opaque media payload | Full for the payload header | Reserved-bit and defined-code policy is documented on `PayloadHeader::validate` |
| SMPTE ST 2022-7 "Seamless Protection Switching of RTP Datagrams" | 2019 (vendored) | Only the `VSID` field it relies on | Not implemented | Spec grounding in `docs/st2022-7-hitless.md` |
| SMPTE ST 2022-5 (FEC) | n/a | Referenced by ST 2022-6 §7.1 for interoperability limits only | Not implemented | |
| IETF RFC 3550 (RTP) | RFC 3550 | RTP fixed header | Not implemented here | Use `rtp-packet` |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | `std::error::Error` impl for `Error`. |
| `serde` | no | `Serialize` on the header and field enums. |

The crate is `#![no_std]` (it uses `alloc`) when default features are off.

## Quick start

Parse the 12-byte payload header from the committed real-capture fixture and write it back:

```rust
use broadcast_common::{Parse, Serialize};
use st2022::PayloadHeader;

// First HBRMT payload header of the 1080i59.94 loopback capture.
let bytes = std::fs::read("../fixtures/st2022/st2022-6-hbrmt-payload-header.bin").unwrap();
let header = PayloadHeader::parse(&bytes).unwrap();

assert_eq!(header.fr_count, 0);
let vsf = header.video_source_format.expect("this header carries a video source format");
println!("map={} frame={} frate={} sample={}",
    vsf.map.name(), vsf.frame.name(), vsf.frate.name(), vsf.sample.name());
println!("clock={} vsid={}", header.clock_frequency.name(), header.vsid.name());

assert_eq!(header.try_to_bytes().unwrap(), bytes);
```

## What is implemented

- `PayloadHeader`: the 4-byte fixed part plus optional 4-byte video source format, 4-byte video timestamp and header extension, with the `validate` rules the spec states.
- `VideoSourceFormat` with typed `MapStructure`, `FrameStructure`, `FrameRate`, `SampleStructure`.
- Typed enums `VideoSourceId`, `TimestampRef`, `Scrambling`, `FecUsage`, `ClockFrequency`. Each has `name()` and `Display`, and a reserved catch-all that keeps the wire value so unknown codes round-trip.
- `HeaderExtension` as a typed TLV view (`items()` yields `ExtensionItem::{Pad, Tlv}`). A malformed TLV chain still round-trips byte-identically and is reported by the item walk.
- Errors: `Error` is `#[non_exhaustive]` with `BufferTooShort`-style context.

## Design notes

Zero-copy (`PayloadHeader<'a>` borrows the extension), symmetric `Parse`/`Serialize`, `#![forbid(unsafe_code)]`, no magic numbers, `#[non_exhaustive]` enums.

## Related crates

Depends on `broadcast-common` only. Use `rtp-packet` for the RTP header, and `media-plane` for the future ST 2022-7 merge. Spec transcriptions are in this crate's `docs/`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
