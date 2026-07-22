# mpeg-ts

[![crates.io](https://img.shields.io/crates/v/mpeg-ts.svg)](https://crates.io/crates/mpeg-ts)
[![docs.rs](https://img.shields.io/docsrs/mpeg-ts)](https://docs.rs/mpeg-ts)
[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)

**MPEG-2 Transport Stream framing for Rust.**

Parses and serializes the ITU-T H.222.0 / ISO/IEC 13818-1 TS packet layer:
188-byte TS packets, adaptation fields, PCR, PSI section reassembly, section
packetisation, and sync recovery. `no_std` + `alloc` — runs on embedded targets
with a heap.

Extracted from `dvb-si` at the 8.0.0 breaking boundary and published
independently so projects that only need raw TS framing do not have to pull in
the full SI/descriptor stack.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Enable `std::error::Error` impls; disable for embedded |
| `serde` | yes     | `Serialize` for packet/section types (serialize-only) |

## Installation

```toml
[dependencies]
mpeg-ts = "0.1"
```

For embedded (`no_std + alloc`):

```toml
[dependencies]
mpeg-ts = { version = "0.1", default-features = false }
```

## Quickstart — feed TS packets into a `SectionReassembler`

```rust
use mpeg_ts::ts::{TsPacket, SectionReassembler};

let mut reasm = SectionReassembler::default();
// Feed 188-byte aligned TS packets; poll for completed PSI sections.
for raw_packet in ts_source {
    let pkt = TsPacket::parse(raw_packet).expect("valid TS packet");
    if let (Some(payload), pusi) = (pkt.payload, pkt.header.pusi) {
        reasm.feed(payload, pusi);
    }
    while let Some(section_bytes) = reasm.pop_section() {
        // section_bytes is a complete PSI section — pass to dvb-si or similar.
        println!("section {} bytes, table_id=0x{:02X}", section_bytes.len(), section_bytes[0]);
    }
}
```

### Typed packet inspection

The [`ScramblingControl`] and [`AdaptationFieldControl`] enums give typed
access to the 2-bit `transport_scrambling_control` and `adaptation_field_control`
fields. Use `scrambling_control()` / `adaptation_field_control()` on either
[`TsHeader`] or [`OwnedTsPacket`]:

```rust
use mpeg_ts::ts::{TsPacket, ScramblingControl, AdaptationFieldControl};
use mpeg_ts::OwnedTsPacket;

// On a borrowed TsPacket:
let pkt = TsPacket::parse(raw_188_bytes)?;
let sc = pkt.header.scrambling_control();
let afc = pkt.header.adaptation_field_control();

// On an owned OwnedTsPacket:
let owned = OwnedTsPacket::parse(raw_array)?;
let sc2 = owned.scrambling_control();
let afc2 = owned.adaptation_field_control();

match sc {
    ScramblingControl::NotScrambled => println!("clear"),
    ScramblingControl::EvenKey | ScramblingControl::OddKey => println!("scrambled"),
    _ => {}
}
# Ok::<(), mpeg_ts::Error>(())
```

### Bulk-walking a TS byte stream

The free helper [`iter_packets`] walks a buffer of concatenated 188-byte packets
without explicit length checks:

```rust
use mpeg_ts::ts::iter_packets;

// Build a buffer of 3 minimal payload-only packets with sync 0x47.
let mut buf = vec![0u8; 188 * 3];
for chunk in buf.chunks_exact_mut(188) {
    chunk[0] = 0x47;      // sync byte
    chunk[3] = 0x10;      // payload_only (adaptation_field_control = 01)
    chunk[4..].fill(0xFF); // stuffing
}
for pkt in iter_packets(&buf) {
    println!("PID: 0x{:04X}", pkt.header.pid);
}
```

See [`examples/iter_packets.rs`](examples/iter_packets.rs) for a full CLI example
that tallies scrambled vs clear packets across a `.ts` file.

### Building & editing packets

The [`OwnedTsPacket`] helpers let you construct, mutate, and round-trip TS
packets without manual byte manipulation:

```rust
use mpeg_ts::ts::{Pcr, TsPacket, TS_PACKET_SIZE};
use mpeg_ts::OwnedTsPacket;

// Build a null packet (PID 0x1FFF) with a given continuity counter.
let null_raw = OwnedTsPacket::null_packet(3);
let pkt = TsPacket::parse(&null_raw)?;
assert_eq!(pkt.header.pid, 0x1FFF);
assert_eq!(pkt.header.continuity_counter, 3);

// Overwrite a continuity counter in-place.
let mut raw = OwnedTsPacket::serialize_with_payload(0x0100, false, 0, &[]);
OwnedTsPacket::set_continuity_counter(&mut raw, 7);
let pkt = TsPacket::parse(&raw)?;
assert_eq!(pkt.header.continuity_counter, 7);

// Shift a PCR bearing packet (requires adaptation field with PCR flag).
let mut raw = [0x47u8; 188];
raw[3] = 0x20; // adaptation-only
raw[4] = 7;    // adaptation_field_length
raw[5] = 0x10; // PCR flag
OwnedTsPacket::set_pcr(&mut raw, Pcr::from_27mhz(27_000_000))?;
let pkt = TsPacket::parse(&raw)?;
assert_eq!(pkt.adaptation_field().unwrap()?.pcr.unwrap().as_27mhz(), 27_000_000);
# Ok::<(), mpeg_ts::Error>(())
```

Run the worked example:
```sh
cargo run -p mpeg-ts --example edit_packet
```

See [`examples/edit_packet.rs`](examples/edit_packet.rs) for a complete walk-through
that builds a PCR-bearing packet, mutates PCR and CC, builds a null
packet, and round-trips a packet header.

## Spec

- **ITU-T H.222.0** (= ISO/IEC 13818-1): §2.4.3.2 (TS packet), §2.4.4 (PSI
  section), §2.4.3.3 (adaptation field), §2.4.3.4 (PCR). All wire layouts are
  cited to the clause in the module doc comments.

## Lineage

This crate was extracted from `dvb-si` at the **8.0.0** release boundary (June
2026). Users migrating from `dvb-si ≤ 7.x` who used the TS framing layer
directly should replace:

| Old (`dvb_si::…`) | New (`mpeg_ts::…`) |
|---|---|
| `ts::TsPacket` | `ts::TsPacket` |
| `ts::SectionReassembler` | `ts::SectionReassembler` |
| `mux::SiMux` / `SectionPacketiser` | `mux::SiMux` / `SectionPacketiser` |
| `resync::TsResync` | `resync::TsResync` |
| `pid::Pid` | `pid::Pid` |

## License

Licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
