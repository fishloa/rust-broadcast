# rtp-packet

[![Crates.io](https://img.shields.io/crates/v/rtp-packet.svg)](https://crates.io/crates/rtp-packet)
[![docs.rs](https://img.shields.io/docsrs/rtp-packet)](https://docs.rs/rtp-packet)

RTP fixed header, CSRC list and generic header extension (RFC 3550 §5.1 / §5.3.1) and the RTCP control packets (§6, module `rtcp`): spec-complete parse and serialize, `no_std`. Optional RFC 8285 extension elements and AV3A-AATF audio payload helpers.

```toml
[dependencies]
rtp-packet = "0.5"
```

## What it does / what it does not

**Does:** parse and build an RTP packet: version (checked `== 2`), padding (the trailing pad-count octet is stripped on parse and re-added on serialize), 0 to 15 CSRC identifiers, marker, payload type, sequence number, timestamp, SSRC, an optional generic header extension, and the payload as a borrowed slice. The `P`, `X` and `CC` bits and the extension length are derived from the typed data, so they cannot disagree with it.

**Does not:** interpret payload formats (H.264 FU-A, AAC-hbr, MPEG-TS-over-RTP and so on live in `transmux` and friends), decode RTCP feedback packets (PLI, REMB and XR are carried opaque; RFC 4585 Generic NACK is decoded in `rist-runtime`), schedule RTCP reports (RFC 3550 §6.2/§6.3), compute jitter or loss statistics, reorder or jitter-buffer packets, or run sockets. It is the one RTP header codec that the rest of the workspace (`transmux`, `dvb-stream`, `multimux`, `webrtc-runtime`, `st291`'s RTP transport) shares instead of each crate hand-rolling its own.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| IETF RFC 3550 "RTP" | RFC 3550 (private repo: `private/specs-verified/ietf/specs/rfc3550_rtp.txt`) | §5.1 fixed header, CSRC list, padding; §5.3.1 generic header extension | Full | Transcribed in `private/specs-verified/rtp/rtp-packet/rtp-header.md` |
| IETF RFC 8285 "A General Mechanism for RTP Header Extensions" | RFC 8285 (vendored) | §4.2 one-byte form (`0xBEDE`), §4.3 two-byte form (`0x100x`) | Full, behind `rfc8285` | `private/specs-verified/rtp/rtp-packet/rfc8285_header_ext.md`; serialization validates every element before writing |
| T/AI 109.7-2024 "AV3A-AATF RTP payload" (clause 10.1) | T/AI 109.7-2024, vendored (`private/specs/t_ai_109_7_2024_avs3_audio_encapsulation_transport.pdf`, md clauses 10.1.1 to 10.1.4) | `aatf_frame()` start, MTU fragmentation and `M`-bit reassembly, SDP `fmtp` parameters | Partial, behind `av3a` | CRC bytes are carried, not verified (T/AI 109.3 Table A.5 names `crc_check` as `rpchof` but states no polynomial or initial value); lossless header typed up to `aatf_error_check()` (the lossless `channel_number` width is an exact spec ambiguity, quoted in `private/specs-verified/rtp/rtp-packet/av3a-aatf-frame-header.md`: the text does not say whether the 8-bit form replaces the nibble or follows a `0xf` nibble, so the bytes after it stay in `AatfFrame::body`); `audio_codec_id` 3..=15 rejected |
| T/AI 109.3-2023 Annex A Tables A.4 to A.7 (AATF frame header) | T/AI 109.3-2023, vendored (English and Chinese editions in `private/specs/`) | General high-rate (0), lossless (1, partial) and general full-rate (2) headers | Partial, behind `av3a` | Same gaps as above |
| IETF RFC 3550 "RTP", RTCP (`rtcp` module) | RFC 3550 | §6.1 compound packets, §6.4.1 SR, §6.4.2 RR, §6.4.1 report block (24-bit signed `cumulative_lost`), §6.5 SDES (CNAME, NAME, EMAIL, PHONE, LOC, TOOL, NOTE, PRIV), §6.6 BYE, §6.7 APP, §6.4.1 profile-specific extensions | Full | SDES PRIV's internal `prefix`/`value` split is not separately typed (the text is kept flat and round-trips). See `private/specs-verified/rtp/rtcp-packet/rtcp.md` |
| IETF RFC 5506 "Reduced-Size RTCP" | RFC 5506 (not vendored) | A compound that starts with a non-SR/RR packet is accepted | Partial | Only the leading-packet relaxation, no feedback semantics |
| IETF RFC 4585 / RFC 5104 / RFC 3611 / REMB draft | n/a | PT 205, 206, 207 and XR | Typed-opaque | Preserved as `rtcp::RtcpPacket::Unknown` |
| IETF RFC 3551 (RTP A/V profile) | RFC 3551 | Static payload type numbers | Not implemented | The payload type is a plain `u8` (max 127) |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`. |
| `serde` | no | `serde::Serialize` on the public types. |
| `rfc8285` | no | The `rfc8285` module: decode and encode one-byte and two-byte header-extension elements. |
| `av3a` | no | The `av3a` module: AATF frame codec, `packetize` / `Av3aDepacketizer`, SDP `Av3aFmtp`. |

## Quick start

Parse a real 324-byte packet (produced by `transmux`'s RTP packetiser over the committed `h264-aac.ts` capture) and write it back byte-for-byte:

```rust
use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

let wire = std::fs::read("../fixtures/rtp/rtp-simple.bin").unwrap();
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

### RTCP

Parse the real Chromium compound (RR + PLI + REMB, 68 bytes), inspect each packet, and write it back unchanged:

```rust
use broadcast_common::{Parse, Serialize};
use rtp_packet::rtcp::{CompoundPacket, RtcpPacket};

let wire = std::fs::read("../fixtures/rtp/chrome-rr-pli-remb.bin").unwrap();
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

Runnable programs: `build_packet`, `parse_packet`, `build_sender_report`, `parse_compound_packet`, and `rfc8285_extensions` (with `--features rfc8285`), via `cargo run -p rtp-packet --example <name>`.

## What is implemented

- `RtpPacket` and `HeaderExtension` (16-bit profile id plus opaque data, with the 32-bit-word length derived).
- Limits as named constants: `RTP_VERSION`, `FIXED_HEADER_LEN`, `MAX_CSRC_COUNT` (15), `MAX_PADDING_COUNT`, `MAX_PAYLOAD_TYPE`.
- `rtcp`: `SenderReport` / `ReceiverReport` (constructors `new` and `with_profile_extensions`) and `ReportBlock`; `SourceDescription`, `SdesChunk`, `SdesItem`, `SdesItemType` with 32-bit chunk padding; `Bye`; `App`; `RtcpPacket` / `RtcpPacketType` for PT dispatch; `CompoundPacket` with byte-exact round-trip. Typed packets validate and strip padding on parse and write `P = 0`; `Unknown` keeps it verbatim. A `cumulative_lost` outside the 24-bit signed range is `rtcp::Error::InvalidValue`, and SDES/BYE text is UTF-8 validated before allocation. `rtcp` has its own `Error`/`Result`, separate from the RTP header ones.
- `rfc8285`: `parse_extensions` returning `ExtensionElements::{OneByte, TwoByte}`, with validated `OneByteId` / `TwoByteId`.
- `av3a`: `AatfFrame`, `AatfFrameHeader`, `packetize`, `Av3aDepacketizer` (capped at 1 MiB by default, `with_max_frame_bytes` to change), `Av3aFmtp`, `CodecNnId`.

## Design notes

Zero-copy `Parse`, symmetric `Serialize`, no magic numbers, `#[non_exhaustive]` `Error`, serializers that check lengths rather than casting, `no_std` + `alloc`. Padding, CSRC and extension cases are tested against spec-table vectors because no public pcap with those fields was available; the simple case is tested against the real fixture.

## Related crates

Depends only on `broadcast-common`. Used by `transmux` (RTP/RTCP packetiser/depacketiser), `rist-runtime` (Generic/Range NACK, RTT echo, compound builders), `dvb-stream`, `webrtc-runtime`, `multimux` (WHEP) and `st291` (ST 2110-40 RTP). The `rtcp` module is the former `rtcp-packet` crate, extracted originally from `transmux::rtcp`; its pre-merge history is in `docs/history/rtcp-packet/CHANGELOG.md`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
