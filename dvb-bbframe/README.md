# dvb-bbframe

[![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe)
[![docs.rs](https://img.shields.io/docsrs/dvb-bbframe)](https://docs.rs/dvb-bbframe)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

ETSI DVB-S2 / S2X / T2 Base-Band Frame (BBFRAME) parser and builder, supporting
both Normal Mode (NM) and High Efficiency Mode (HEM).

A BBFRAME is the unit of payload carried in a DVB-S2/S2X/T2 baseband stream: a
10-byte BBHEADER followed by a data field of user packets (a Transport Stream or
Generic Stream). This crate parses and rebuilds the header and walks the data
field; it does **not** implement the physical layer (LDPC/BCH coding, modulation).
GSE payloads (Generic Stream Encapsulation) are out of scope — hand the data
field to the third-party [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## Quick start

```rust
use dvb_bbframe::header::{Bbheader, Mode};
use dvb_bbframe::pump::BbframePump;

// Parse a 10-byte BBHEADER directly:
let hdr = Bbheader::parse(frame)?;
assert_eq!(hdr.mode, Mode::Normal);
println!("UPL={} DFL={} SYNC=0x{:02X}", hdr.upl, hdr.dfl, hdr.sync);

// Or use the pump for per-PLP BBFrame→inner-TS extraction:
let mut pump = BbframePump::new();
let inner_ts_packets = pump.feed(5 /* plp_id */, &df_bytes);
for pkt in inner_ts_packets {
    // 188-byte inner TS packets, sync byte restored
}
```

## What's implemented

### BBHEADER (10 bytes, EN 302 307-1 / EN 302 755)

All fields from the `Bbheader` struct:

| Field | Width | Notes |
|-------|-------|-------|
| `matype.ts_gs` | 2 bits | `TsGs` enum: `Ts`, `Gfps`, `Gcs`, `Gse` |
| `matype.sis` | 1 bit | Single/Multi input stream |
| `matype.ccm` | 1 bit | CCM/ACM flag |
| `matype.issyi` | 1 bit | Input Stream Synchronization Indicator active |
| `matype.npd` | 1 bit | Null Packet Deletion active |
| `matype.ext` | 2 bits | Roll-off / reserved (`RollOff` accessor for S2/S2X) |
| `matype.isi` | 8 bits | Input Stream Identifier (MIS only) |
| `upl` | 16 bits | User Packet Length in bits (NM only; 0 in HEM) |
| `sync` | 8 bits | UP sync byte copy (NM only) |
| `dfl` | 16 bits | Data Field Length in bits |
| `syncd` | 16 bits | Bit offset to first complete UP in data field |
| `mode` | CRC-8 | `Mode` enum: `Normal` (0) or `HighEfficiency` (1), recovered via `crc8(hdr[0..9]) ^ hdr[9]` |
| `issy_in_header` | 3 bytes | ISSY bytes (HEM only; `None` in NM) |

`Bbheader::issy()` decodes `issy_in_header` into an `Issy` enum.

### ISSY decoding (EN 302 755 Annex C / EN 302 307-1 Annex D)

`Issy`, `SignallingKind`, and `BufsUnit` cover all three ISSY forms:

| Form | Prefix | Result |
|------|--------|--------|
| ISCR short | `bit[7]=0` | `Issy::IscrShort(u16)` — 15-bit ISCR |
| ISCR long | `bit[7:6]=10` | `Issy::IscrLong(u32)` — 22-bit ISCR |
| Signalling | `bit[7:6]=11` | `Issy::Signalling(SignallingKind)` |

`SignallingKind` variants: `Bufs { bufs, units }`, `Tto { tto_e, tto_m, tto_l }`,
`BufStat { bufstat, units }` (DVB-S2 Annex D), `Reserved(u32)`. Decoded accessors:
`bufs_bits()`, `bufs_bytes()`, `bufstat_bits()`, `bufstat_bytes()`, `tto_t_over_256()`.

### User-packet extraction (EN 302 755 §5.1.8)

| Type | Description |
|------|-------------|
| `NmTsIter` | Iterator over NM UPs (188-byte stride; CRC-8 byte replaced with sync 0x47) |
| `HemTsIter` | Iterator over HEM UPs (187-byte stride; sync byte prepended; DNP skipped when NPD active) |
| `UpIter` | Runtime-dispatched enum wrapping either iterator |
| `up_iter(data, bbheader)` | Constructs the right iterator from the parsed header |
| `CarryOverExtractor` | Stateful cross-boundary reassembler; `feed_nm` / `feed_hem` (allocating) and `feed_nm_into` / `feed_hem_into` (buffer-reuse) |

`SYNCD = 0xFFFF` (no UP starts in this data field) is handled correctly.

### BbframePump

`BbframePump` packages the whole BBHEADER-parse → mode-detect → `CarryOverExtractor`
chain for multi-PLP streams. Feed `(plp_id, df_bytes)` pairs; the pump keeps
independent per-PLP carry-over state and returns completed 188-byte inner TS packets.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | **on** | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | off | Serialize-only (`serde::Serialize`) on `Bbheader`, `Matype`, `TsGs`, `Mode`, `Issy`, `SignallingKind`, `BufsUnit`, `RollOff`. |
| `yoke` | off | `yoke::Yokeable` on `NmTsIter` and `HemTsIter`. |

## MSRV

Rust **1.81**.

## Authoritative references

- ETSI EN 302 307-1 (DVB-S2) / EN 302 307-2 (DVB-S2X)
- ETSI EN 302 755 (DVB-T2) — §5.1.7 (BBHEADER/HEM), §5.1.8 (UP carriage), Annex C (ISSY), Annex F (CRC-8)

The structured spec reference is under [`docs/`](docs/); the canonical PDFs are
vendored in the workspace `specs/` directory.

## Examples

Run with `cargo run -p dvb-bbframe --example <name>`:

- **`parse_bbheader`** — parse a single DVB-S2 BBHEADER (with a valid Normal-Mode CRC-8).
- **`walk_capture`** — reassemble BBFrames from a real DVB-S2 capture and parse every header.

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
