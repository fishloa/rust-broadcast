# dvb-t2mi

[![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi)
[![docs.rs](https://img.shields.io/docsrs/dvb-t2mi)](https://docs.rs/dvb-t2mi)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Complete, spec-compliant **ETSI TS 102 773 v1.4.1** DVB-T2 Modulator Interface (T2-MI) parser and builder in Rust.

## What is T2-MI?

T2-MI is the protocol between a DVB-T2 Gateway and a DVB-T2 modulator. It carries:

- **Baseband Frames (BBFRAME)** — the actual DVB-T2 user data
- **L1 signalling** — configuration for each T2 frame (L1-pre and L1-post)
- **Auxiliary I/Q data** — pilot and correction cell values
- **DVB-T2 timestamps** — SFN synchronisation
- **Future Extension Frames (FEF)** — composite signal support (e.g. T2-Base + T2-Lite)

Each T2-MI packet is a 6-byte header + variable payload + 4-byte CRC-32, encapsulated in MPEG-2 TS via data piping.

## Quick start

```rust
use dvb_t2mi::pump::T2miPump;
use dvb_t2mi::payload::AnyPayload;

let mut pump = T2miPump::new(0x0006);       // T2-MI PID from the PMT
for packet in ts_packets {                  // each aligned 188-byte packet
    for event in pump.feed_ts(packet) {     // CRC-valid packets only
        match event.payload()? {
            AnyPayload::Bbframe(bb)   => println!("BBFrame plp_id={}", bb.plp_id),
            AnyPayload::L1Current(l1) => { /* l1.l1_pre(), l1.l1_post() */ }
            AnyPayload::Timestamp(ts) => { let _ = ts; }
            AnyPayload::Unknown { packet_type, .. } => eprintln!("0x{packet_type:02X}"),
            _ => {}
        }
    }
}
```

For un-encapsulated `.t2mi` byte streams use `T2miPump::raw()` + `feed_raw`. The
[`dvb-tools t2mi`](../dvb-tools/) CLI is the complete wrapper
(`cargo run -p dvb-tools -- t2mi file.ts [--pid 0xNNN|raw] [--inner] [--plp N]`).

### Inner-TS recovery in one call

[`inner_ts::InnerTsRecovery`] folds the full chain (T2miPump → `AnyPayload::Bbframe` →
`Bbheader` → `CarryOverExtractor`, NM/HEM + carry-over) into a single
feed-and-collect driver — feed outer TS packets, get inner TS packets out:

```rust
# #[cfg(feature = "ts")] {
use dvb_t2mi::inner_ts::InnerTsRecovery;

let mut rec = InnerTsRecovery::new(0x1000); // the T2-MI PID
for outer in ts_packets() {                 // 188-byte outer TS packets
    for inner in rec.feed(&outer) {          // recovered inner TS packets
        assert_eq!(inner[0], 0x47);
    }
}
# fn ts_packets() -> Vec<[u8; 188]> { vec![] }
# }
```

`InnerTsRecovery::with_plp(pid, plp_id)` filters to a single PLP.

## What's implemented

### Packet types — all 12 (ETSI TS 102 773 Table 1)

All 12 are in the `declare_payloads!` list in `payload/any.rs`; each has a
`Parse` + `Serialize` impl and round-trip tests. The `AnyPayload` dispatcher
routes every packet type to its typed struct; unknown/reserved values fall
through to `AnyPayload::Unknown` with raw bytes preserved.

| Value | Packet type | Struct |
|-------|-------------|--------|
| 0x00 | Baseband Frame — §5.2.1 | `BbframePayload` |
| 0x01 | Auxiliary stream I/Q data — §5.2.2 | `AuxIqPayload` |
| 0x02 | Arbitrary cell insertion — §5.2.3 | `ArbitraryCellsPayload` |
| 0x10 | L1-current signalling — §5.2.4 | `L1CurrentPayload` |
| 0x11 | L1-future signalling — §5.2.5 | `L1FuturePayload` |
| 0x12 | P2 bias balancing cells — §5.2.6 | `P2BiasPayload` |
| 0x20 | DVB-T2 timestamp — §5.2.7 | `T2TimestampPayload` |
| 0x21 | Individual addressing — §5.2.8 | `IndividualAddressingPayload` |
| 0x30 | FEF part: Null — §5.2.9 | `FefNullPayload` |
| 0x31 | FEF part: I/Q data — §5.2.10 | `FefIqPayload` |
| 0x32 | FEF part: composite — §5.2.11 | `FefCompositePayload` |
| 0x33 | FEF sub-part — §5.2.12 | `FefSubPartPayload` |

### L1 signalling (EN 302 755 §7.2)

`L1CurrentPayload` and `L1FuturePayload` expose typed `l1_pre()` / `l1_post()`
accessors that parse the L1-pre and L1-post signalling structures directly from
the payload bytes. Decoded enums (`L1PreType`, `Papr`, `S1Field`, `S2Field`,
`FftMode`, `Gi`, `Pp`, `Mimo`, etc.) live in `payload::l1::enums`.

### DVB-T2 timestamp decoded accessors

`T2TimestampPayload` exposes `emission_offset()` (per-bandwidth `T_sub` units,
§5.2.7 Table 4) and the `is_null` / `is_relative` flags. The `chrono` feature
adds a `utc_emission_time()` accessor returning `DateTime<Utc>`.

### PLP filter

`T2miPump::new(pid)` passes all PLPs through; `InnerTsRecovery::with_plp(pid,
plp_id)` restricts the inner-TS recovery to a single PLP. The `T2miEvent`
exposes `plp_id()` for caller-side filtering on the individual BBFrame events.

### Private / custom packet types

`PayloadRegistry` lets you register owned types for packet_type values not in
`PacketType` (or override built-in ones). Call `event.payload_with(&reg)` instead
of `event.payload()`; the registry's parser wins, producing `AnyPayload::Other {
packet_type, value }` where `value` can be downcast to your concrete type.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | **on** | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `ts` | **on** | `T2miPump` feed-and-iterate front door + `InnerTsRecovery`; pulls in `bytes` and `dvb-bbframe`. |
| `serde` | off | Serialize-only (`serde::Serialize`) on all header and payload types; no Deserialize. |
| `chrono` | off | Decoded UTC emission-time accessor on `T2TimestampPayload`. |
| `yoke` | off | `yoke::Yokeable` on zero-copy payload view types. |

## MSRV

Rust **1.81**.

## References

- [ETSI TS 102 773 v1.4.1](https://www.etsi.org/deliver/etsi_ts/102700_102799/102773/) — DVB-T2 Modulator Interface (T2-MI)
- [ETSI EN 302 755 v1.4.1](https://www.etsi.org/deliver/etsi_en/302700_302799/302755/) — DVB-T2 Frame structure, channel coding and modulation

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
