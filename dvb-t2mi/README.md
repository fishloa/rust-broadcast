# dvb_t2mi

[![crates.io](https://img.shields.io/badge/version-0.1.0-blue.svg)](https://crates.io/crates/dvb_t2mi)
[![docs.rs](https://img.shields.io/badge/docs.rs-dvb__t2mi-green.svg)](https://docs.rs/dvb_t2mi)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Complete, spec-compliant **ETSI TS 102 773 v1.4.1** DVB-T2 Modulator Interface (T2-MI) parser and builder in Rust.

## What is T2-MI?

T2-MI is the protocol between a DVB-T2 Gateway and a DVB-T2 modulator. It carries:

- **Baseband Frames (BBFRAME)** — the actual DVB-T2 user data
- **L1 signalling** — configuration for each T2 frame
- **Auxiliary I/Q data** — pilot and correction cell values
- **DVB-T2 timestamps** — SFN synchronisation
- **Future Extension Frames (FEF)** — composite signal support (e.g. T2-Base + T2-Lite)

Each T2-MI packet is 6-byte header + variable payload + 4-byte CRC-32, encapsulated in MPEG-2 TS via data piping.

## Features

| Feature | Description | Status |
|---------|-------------|--------|
| Header parsing | 6-byte T2-MI header with bit-accurate field extraction | ✅ |
| All 12 packet types | 0x00 (BBFrame) through 0x33 (FEF sub-part) | ✅ |
| Payload parsing | Every payload type with Parse/Serialize symmetry | ✅ |
| CRC-32 validation | MPEG-2 Annex A polynomial (0x04C1_1DB7) | ✅ |
| TS reassembler | MPEG-2 TS decapsulation per §6.1.1 (PUSI + pointer_field) | ✅ |
| Zero-copy fan-out | All parsed payloads return `&[u8]` slices | ✅ |

### Cargo features

| Feature | Default | Description |
|---------|---------|-------------|
| `ts` | on | MPEG-2 TS reassembler (`ts::PacketReassembler`); pulls in `bytes` |
| `serde` | off | `Serialize`/`Deserialize` derives on the header and all payload types |

`no_std` is not yet supported (`thiserror` requires `std`).

## Spec Compliance

All types implement the `Parse` and `Serialize` traits defined in this crate, with round-trip tests for every payload. Reserved bits are validated and must be zero.

| Clause | Packet Type | Implementation |
|--------|-------------|----------------|
| §5.1 | T2-MI header (6 bytes) | [`packet::Header`] |
| §5.2.1 | Baseband Frame (0x00) | [`payload::BbframePayload`] |
| §5.2.2 | Auxiliary I/Q data (0x01) | [`payload::AuxIqPayload`] |
| §5.2.3 | Arbitrary cell insertion (0x02) | [`payload::ArbitraryCellsPayload`] |
| §5.2.4 | L1-current signalling (0x10) | [`payload::L1CurrentPayload`] |
| §5.2.5 | L1-future signalling (0x11) | [`payload::L1FuturePayload`] |
| §5.2.6 | P2 bias balancing cells (0x12) | [`payload::P2BiasPayload`] |
| §5.2.7 | DVB-T2 timestamp (0x20) | [`payload::T2TimestampPayload`] |
| §5.2.8 | Individual addressing (0x21) | [`payload::IndividualAddressingPayload`] |
| §5.2.9 | FEF part: Null (0x30) | [`payload::FefNullPayload`] |
| §5.2.10 | FEF part: I/Q data (0x31) | [`payload::FefIqPayload`] |
| §5.2.11 | FEF part: composite (0x32) | [`payload::FefCompositePayload`] |
| §5.2.12 | FEF sub-part (0x33) | [`payload::FefSubPartPayload`] |
| §6.1 | TS encapsulation | [`ts::PacketReassembler`] |
| Annex A | CRC-32 polynomial | [`crc::crc32`], [`crc::validate_crc`] |

## Quick Start

### Parse a T2-MI packet header

```rust
use dvb_t2mi::packet::{Header, PacketType};
use dvb_common::Parse;

let buf: &[u8] = &[
    0x00, // packet_type = Baseband Frame
    0x01, // packet_count
    0x10, // superframe_idx=1, t2mi_stream_id=0
    0x00, // RFU (8 bits, zero)
    0x00, 0x18, // payload_len_bits = 24 (3 bytes)
    0x00, 0x03, 0x0A, // payload: frame_idx, plp_id, intl_frame_start
];

let hdr = Header::parse(buf)?;
assert_eq!(hdr.packet_type, PacketType::BasebandFrame);
assert_eq!(hdr.packet_count, 1);
let payload = hdr.payload_bytes(buf)?; // the 3 payload bytes
```

### Validate CRC-32

```rust
use dvb_common::crc32_mpeg2::compute as crc32;
use dvb_t2mi::crc::validate_crc;

let mut packet = Vec::from(header_bytes);
packet.extend_from_slice(&payload_bytes);
let crc = crc32(&packet);
packet.extend_from_slice(&crc.to_be_bytes());

validate_crc(&packet)?; // Ok(())
```

### Reassemble from MPEG-2 TS

```rust
use dvb_t2mi::ts::PacketReassembler;

let mut reasm = PacketReassembler::new();

// Feed each incoming TS packet (184-byte payload, PUSI flag, PID)
for (ts_payload, pusi) in ts_packets {
    reasm.feed(&ts_payload, pusi, 0x0006);
}

// Drain completed T2-MI packets
while let Some(t2mi_packet) = reasm.pop_packet() {
    // Parse header, validate CRC, extract BBFRAME, etc.
    let hdr = Header::parse(&t2mi_packet)?;
    validate_crc(&t2mi_packet)?;
}
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    MPEG-2 TS packets                      │
│              (data piping, PID 0x0006)                    │
└──────────────────────────┬──────────────────────────────┘
                           │
                    ┌──────▼──────┐
                    │  PUSI flag  │
                    │  ptr_field  │
                    └──────┬──────┘
                           │
              ┌────────────▼────────────┐
              │  ts::PacketReassembler  │
              │  (§6.1.1 decapsulation) │
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │     T2-MI packets        │
              │  (6B hdr + payload + CRC) │
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │   packet::Header::parse  │
              │   crc::validate_crc      │
              └────────────┬────────────┘
                           │
              ┌────────────▼────────────┐
              │  payload::*::parse       │
              │  (Bbframe, L1, FEF, etc) │
              └─────────────────────────┘
```

## Module Structure

| Module | Purpose | Feature |
|--------|---------|---------|
| `packet` | T2-MI header (6 bytes) + `PacketType` enum | always |
| `payload` | Each of the 12 payload types | always |
| `payload::bbframe` | Type 0x00 — Baseband Frame | always |
| `payload::aux_iq` | Type 0x01 — Auxiliary I/Q data | always |
| `payload::arbitrary_cells` | Type 0x02 — Arbitrary cell insertion | always |
| `payload::l1_current` | Type 0x10 — L1-current signalling | always |
| `payload::l1_future` | Type 0x11 — L1-future signalling | always |
| `payload::p2_bias` | Type 0x12 — P2 bias balancing cells | always |
| `payload::timestamp` | Type 0x20 — DVB-T2 timestamps | always |
| `payload::individual_addressing` | Type 0x21 — Per-transmitter functions | always |
| `payload::fef_null` | Type 0x30 — FEF Null, `S1Field` enum | always |
| `payload::fef_iq` | Type 0x31 — FEF I/Q data | always |
| `payload::fef_composite` | Type 0x32 — FEF composite info | always |
| `payload::fef_subpart` | Type 0x33 — FEF sub-parts | always |
| `crc` | MPEG-2 CRC-32 (Annex A polynomial) | always |
| `ts` | MPEG-2 TS reassembler (PUSI + pointer_field) | `ts` (default) |
| `error` | `Error` enum with `thiserror` | always |

The `Parse` / `Serialize` contracts live in the `dvb_common` crate.

## What's NOT in this crate

This crate implements **only the T2-MI protocol** (ETSI TS 102 773). It does **not** include:

- **BBHeader parsing** — the 10-byte DVB-T2 BBHEADER is defined in EN 302 755, handled by the consuming application's adaptor layer.
- **HEM (High Efficiency Mode)** detection — an adaptor-layer concern (CRC-8 init-XOR + 187/188-byte user-packet stride), not part of the T2-MI protocol itself.
- **DVB-T2 physical layer** — LDPC coding, OFDM cell mapping, time interleaving are all outside scope.
- **L1 signalling parsing** — the raw L1-current/L1-future bytes are passed through uninterpreted; a DVB-T2 modulator consumes them directly.

## References

- [ETSI TS 102 773 v1.4.1](https://www.etsi.org/deliver/etsi_ts/102700_102799/102773/) — DVB-T2 Modulator Interface (T2-MI)
- [ETSI EN 302 755 v1.4.1](https://www.etsi.org/deliver/etsi_en/302700_302799/302755/) — DVB-T2 Frame structure, channel coding and modulation
- [ETSI EN 301 192](https://www.etsi.org/deliver/etsi_en/301100_301199/301192/) — DVB Data piping (MPEG-2 TS encapsulation)

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
