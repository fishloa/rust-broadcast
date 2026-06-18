# dvb-pes

[![crates.io](https://img.shields.io/crates/v/dvb-pes.svg)](https://crates.io/crates/dvb-pes)
[![docs.rs](https://img.shields.io/docsrs/dvb-pes)](https://docs.rs/dvb-pes)
[![MSRV](https://img.shields.io/badge/MSRV-1.81-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/dvb-pes.svg)](#license)

**PES (Packetized Elementary Stream) depacketization + PTS/DTS** — the sublayer
between an MPEG-TS packet layer and an elementary-stream consumer. Per ISO/IEC
13818-1 (Rec. ITU-T H.222.0) §2.4.3.6 / §2.4.3.7.

`#![no_std]` (+ `alloc`), depends only on `dvb-common`, WASM-clean. Pairs with
[`dvb-si`](https://crates.io/crates/dvb-si) for the TS/PSI layer:

```text
TsPacket payload + payload_unit_start ──► dvb-pes ──► PesPacket { stream_id, pts, dts, payload }
```

## Quickstart

```rust
use dvb_pes::{PesPacket, StreamId};

let bytes = [
    0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05,
    0x21, 0x00, 0x01, 0x00, 0x01, // PTS = 0
    0xAA, 0xBB,                   // ES payload
];
let pkt = PesPacket::parse(&bytes)?;
assert_eq!(pkt.stream_id, StreamId(0xE0));
assert!(pkt.stream_id.is_video());
assert_eq!(pkt.header.unwrap().pts.unwrap().ticks(), 0);
assert_eq!(pkt.payload, &[0xAA, 0xBB]);
# Ok::<(), dvb_pes::Error>(())
```

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | ✅ | Link `std`. Off → `#![no_std]` + `alloc`. |
| `serde` | – | `serde::Serialize` on the public types. |

## Scope

In: PES packet header, `stream_id`, `PES_packet_length` (incl. unbounded video
`0`), the optional header flags, and **PTS/DTS** (33-bit @ 90 kHz). Out:
elementary-stream codec bitstream parsing (the consumer's job); network transport
(see `dvb-stream`).

## Examples

Run with `cargo run -p dvb-pes --example <name>`:

- **`parse_pes_packet`** — parse one PES packet from raw bytes (stream_id + PTS + payload).
- **`extract_pts`** — depacketize a real capture, reassemble PES on a PID, and report the PTS timeline.

## License

MIT OR Apache-2.0.
