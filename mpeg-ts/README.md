# mpeg-ts

[![crates.io](https://img.shields.io/crates/v/mpeg-ts.svg)](https://crates.io/crates/mpeg-ts)
[![docs.rs](https://img.shields.io/docsrs/mpeg-ts)](https://docs.rs/mpeg-ts)
[![CI](https://github.com/fishloa/rust-dvb/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-dvb/actions)

**MPEG-2 Transport Stream framing for Rust.**

Parses and serializes the ITU-T H.222.0 / ISO/IEC 13818-1 TS packet layer:
188-byte TS packets, adaptation fields, PCR, PSI section reassembly, section
packetization, and sync recovery. `no_std` + `alloc` — runs on embedded targets
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

See [`examples/demux.rs`](examples/demux.rs) for a runnable end-to-end example.

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
| `mux::SiMux` / `SectionPacketizer` | `mux::SiMux` / `SectionPacketizer` |
| `resync::TsResync` | `resync::TsResync` |
| `pid::Pid` | `pid::Pid` |

## License

Licensed under either of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE), at your option.
