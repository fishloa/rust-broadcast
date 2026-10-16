# mpeg-pes

[![crates.io](https://img.shields.io/crates/v/mpeg-pes.svg)](https://crates.io/crates/mpeg-pes)
[![docs.rs](https://img.shields.io/docsrs/mpeg-pes)](https://docs.rs/mpeg-pes)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/mpeg-pes.svg)](#related-crates-and-license)

**PES (Packetized Elementary Stream) packet parse/serialize with PTS/DTS: the layer between MPEG-TS/PS packets and an elementary-stream consumer.**

## What it does / what it does not

Feed it the payload bytes of one PID (split on `payload_unit_start_indicator`)
and it yields typed PES packets: `stream_id`, the optional PES header (flags,
PTS/DTS, ESCR, ES rate, DSM trick mode, CRC, PES extension) and the
elementary-stream payload. Every field of the PES header syntax is kept so a
packet re-serializes byte-for-byte. It is used by demuxers, analyzers and
repair tools, and underlies `mpeg-ps` and `transmux`.

It does not parse the elementary stream itself (H.264/AAC/... bitstreams are the
consumer's job), does not do TS framing (see `mpeg-ts`) or network transport
(see `dvb-stream`, which frames TS, not PES), and does not parse MPEG-1 PES (rejected with
`Error::BadHeaderMarker`).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1, *Systems* | H.222.0 v9 (08/2023), verified against the vendored copy | §2.4.3.6 PES packet, §2.4.3.7 semantics, Table 2-21 (all optional fields, PES extension, P-STD buffer, program packet sequence counter, trick modes), stream_id assignments, PTS/DTS 33-bit @ 90 kHz, ESCR 27 MHz | Full for the PES layer | Unbounded video PES (`PES_packet_length` 0) supported by `PesAssembler`. |
| ETSI TS 101 154 | V2.10.1 | §4.1.8 AVS3 `stream_id_extension` carriage | Partial: stream-id-extension constants and accessor only | Codec carriage rules themselves are out of scope. |
| T/AI 109.7 (AVS3 system) | clause 8.1.2 as cited in the code; edition unverified (not vendored) | `stream_id_extension` values for AVS3 video/audio | Partial | |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std` (`std::error::Error`); off gives `#![no_std]` + `alloc` |
| `serde` | no | `serde::Serialize` on the public types |

Depends only on `broadcast-common`; WASM-clean.

```toml
[dependencies]
mpeg-pes = "0.5"
```

## Quick start

```rust
use mpeg_pes::{PesAssembler, PesPacket, StreamId};

// start code, stream_id 0xE0 (video), length, header (PTS only), payload
let bytes = [
    0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05,
    0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0
    0xAA, 0xBB,                   // ES payload
];
let pkt = PesPacket::parse(&bytes)?;
assert_eq!(pkt.stream_id, StreamId(0xE0));
assert!(pkt.stream_id.is_video());
assert_eq!(pkt.header.as_ref().unwrap().pts.unwrap().ticks(), 0);
assert_eq!(pkt.payload, &[0xAA, 0xBB]);

// Byte-exact re-serialization.
let mut out = vec![0u8; pkt.serialized_len()];
pkt.serialize_into(&mut out)?;
assert_eq!(out, bytes);

// Reassemble from TS payload chunks: `true` marks a payload_unit_start packet.
let mut asm = PesAssembler::new();
assert!(asm.feed(true, &bytes[..8]).is_none());
assert!(asm.feed(false, &bytes[8..]).is_none());
let unit = asm.flush().expect("buffered PES packet");
assert_eq!(unit, bytes);
# Ok::<(), mpeg_pes::Error>(())
```

Examples (`cargo run -p mpeg-pes --example <name>`): `parse_pes_packet`,
`extract_pts` (depacketize a real capture and print the PTS timeline),
`build_pes_header` (typed PTS+DTS/ESCR/DSM trick mode, serialize, re-parse).

## What is implemented

| Area | Public API |
|---|---|
| Packet | `PesPacket<'a>` (`parse`, `serialized_len`, `serialize_into`), `PesHeader<'a>`, `PACKET_START_CODE_PREFIX` |
| Header fields | `Escr`, `TrickMode`, `AdditionalCopyInfo`, `ProgramPacketSequenceCounter`, `PStdBuffer`, `PesExtension<'a>`, `PesExtensionField<'a>`, `FREEZE_FRAME_RESERVED_VALUE` |
| Timestamps | `Pts`, `Dts` (33-bit @ 90 kHz; `ticks()`, `seconds()`, `to_field_bytes`/`from_field_bytes`) |
| Stream ids | `StreamId` (`PROGRAM_STREAM_MAP`, `PADDING_STREAM`, `PRIVATE_STREAM_2`, `EXTENDED_STREAM_ID`, `has_optional_header`, `is_audio`, `is_video`), `STREAM_ID_EXTENSION_AVS3_VIDEO`, `STREAM_ID_EXTENSION_AVS3_AUDIO` |
| Reassembly | `PesAssembler` (`new`, `with_max_unit_size`, `feed`, `flush`): PUSI-delimited accumulation, size-capped (default 16 MiB, shared with `mpeg-ts` via `broadcast-common`) |
| Errors | `Error`, `Result` |

## Design notes

- **Zero-copy and symmetric.** Parsed packets borrow the input; every type has
  `serialize_into`; `tests/pes_header_roundtrip_exhaustive.rs` re-serializes
  every PES packet of every committed `.ts` fixture.
- **Nothing dropped.** `header_stuffing` (the bytes, not a length),
  `es_rate_marker_bits` and `PesExtension::reserved_bits` are preserved
  (`#[non_exhaustive]` types with `new()`/`Default`).
- **Strict where it matters.** A first flag byte other than `'10'` is
  `Error::BadHeaderMarker`; a `PES_header_data_length` that overruns a bounded
  `PES_packet_length` is an error; a `P-STD_buffer_size` over 13 bits is
  `Error::FieldOverflow`, never masked.
- **Shared clock code.** Marker-bit PTS/DTS/ESCR packing is
  `broadcast_common::clock33`.
- `no_std` + `alloc`, `#![warn(missing_docs)]`; label convention
  (`name()`/`Display`) guarded by `tests/label_coverage.rs`.

## Related crates and license

Depends on `broadcast-common`. Used with `mpeg-ts` (TS payloads in) and
`mpeg-ps` (program-stream PES), and consumed by `transmux`, `ts-fix`, `media-doctor`, `caption-convert` and `dvb-subtitle`.
MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
