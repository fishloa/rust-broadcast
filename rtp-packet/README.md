# rtp-packet

[![Crates.io](https://img.shields.io/crates/v/rtp-packet.svg)](https://crates.io/crates/rtp-packet)
[![docs.rs](https://img.shields.io/docsrs/rtp-packet)](https://docs.rs/rtp-packet)

RTP fixed header, CSRC list and generic header extension (RFC 3550 §5.1 / §5.3.1): spec-complete parse and serialize, `no_std`. Optional RFC 8285 extension elements and AV3A-AATF audio payload helpers.

```toml
[dependencies]
rtp-packet = "0.4"
```

## What it does / what it does not

**Does:** parse and build an RTP packet: version (checked `== 2`), padding (the trailing pad-count octet is stripped on parse and re-added on serialize), 0 to 15 CSRC identifiers, marker, payload type, sequence number, timestamp, SSRC, an optional generic header extension, and the payload as a borrowed slice. The `P`, `X` and `CC` bits and the extension length are derived from the typed data, so they cannot disagree with it.

**Does not:** interpret payload formats (H.264 FU-A, AAC-hbr, MPEG-TS-over-RTP and so on live in `transmux` and friends), do RTCP (use `rtcp-packet`), reorder or jitter-buffer packets, or run sockets. It is the one RTP header codec that the rest of the workspace (`transmux`, `dvb-stream`, `multimux`, `webrtc-runtime`, `st291`'s RTP transport) shares instead of each crate hand-rolling its own.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF RFC 3550 "RTP" | RFC 3550 (vendored `specs/rfc3550_rtp.txt`) | §5.1 fixed header, CSRC list, padding; §5.3.1 generic header extension | Full | Transcribed in `docs/rtp-header.md` |
| IETF RFC 8285 "A General Mechanism for RTP Header Extensions" | RFC 8285 (vendored) | §4.2 one-byte form (`0xBEDE`), §4.3 two-byte form (`0x100x`) | Full, behind `rfc8285` | `docs/rfc8285_header_ext.md`; serialization validates every element before writing |
| T/AI 109.7-2024 "AV3A-AATF RTP payload" (clause 10.1) | 2024 (not in the vendored index, edition per the crate's source citations) | `aatf_frame()` start, MTU fragmentation and `M`-bit reassembly, SDP `fmtp` parameters | Partial, behind `av3a` | CRC bytes are carried, not verified (polynomial not in the vendored extract); lossless header typed up to `aatf_error_check()`; `audio_codec_id` 3..=15 rejected |
| T/AI 109.3-2023 Annex A Tables A.4 to A.7 (AATF frame header) | 2023 (as cited in the source) | General high-rate (0), lossless (1, partial) and general full-rate (2) headers | Partial, behind `av3a` | Same gaps as above |
| IETF RFC 3551 (RTP A/V profile) | RFC 3551 | Static payload type numbers | Not implemented | The payload type is a plain `u8` (max 127) |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`. |
| `serde` | no | `serde::Serialize` on the public types. |
| `rfc8285` | no | The `rfc8285` module: decode and encode one-byte and two-byte header-extension elements. |
| `av3a` | no | The `av3a` module: AATF frame codec, `packetize` / `Av3aDepacketizer`, SDP `Av3aFmtp`. |

## Quick start

Parse a real 324-byte packet (produced by `transmux`'s RTP packetiser over the committed `h264_aac.ts` capture) and write it back byte-for-byte:

```rust
use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

let wire = std::fs::read("tests/fixtures/rtp_simple.bin").unwrap();
let pkt = RtpPacket::parse(&wire).unwrap();
assert!(pkt.extension.is_none() && pkt.csrc.is_empty());
println!("pt={} seq={} ts={} ssrc={:#x}",
    pkt.payload_type, pkt.sequence_number, pkt.timestamp, pkt.ssrc);
assert_eq!(pkt.try_to_bytes().unwrap(), wire);

// Build one from fields.
let out = RtpPacket {
    marker: true,
    payload_type: 96,
    sequence_number: 1,
    timestamp: 3600,
    ssrc: 0x1234_5678,
    csrc: vec![],
    extension: None,
    padding: None,
    payload: &[0xDE, 0xAD, 0xBE, 0xEF],
};
let bytes = out.try_to_bytes().unwrap();
assert_eq!(RtpPacket::parse(&bytes).unwrap(), out);
```

Runnable programs: `build_packet`, `parse_packet`, and `rfc8285_extensions` (with `--features rfc8285`), via `cargo run -p rtp-packet --example <name>`.

## What is implemented

- `RtpPacket` and `HeaderExtension` (16-bit profile id plus opaque data, with the 32-bit-word length derived).
- Limits as named constants: `RTP_VERSION`, `FIXED_HEADER_LEN`, `MAX_CSRC_COUNT` (15), `MAX_PADDING_COUNT`, `MAX_PAYLOAD_TYPE`.
- `rfc8285`: `parse_extensions` returning `ExtensionElements::{OneByte, TwoByte}`, with validated `OneByteId` / `TwoByteId`.
- `av3a`: `AatfFrame`, `AatfFrameHeader`, `packetize`, `Av3aDepacketizer` (capped at 1 MiB by default, `with_max_frame_bytes` to change), `Av3aFmtp`, `CodecNnId`, `is_dynamic_payload_type`.

## Design notes

Zero-copy `Parse`, symmetric `Serialize`, no magic numbers, `#[non_exhaustive]` `Error`, serializers that check lengths rather than casting, `no_std` + `alloc`. Padding, CSRC and extension cases are tested against spec-table vectors because no public pcap with those fields was available; the simple case is tested against the real fixture.

## Related crates

Depends only on `broadcast-common`. Used by `transmux` (RTP packetiser/depacketiser), `dvb-stream`, `webrtc-runtime`, `multimux` (WHEP) and `st291` (ST 2110-40 RTP). `rtcp-packet` is its control-plane companion.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
