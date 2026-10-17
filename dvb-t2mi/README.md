# dvb-t2mi

[![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi)
[![docs.rs](https://img.shields.io/docsrs/dvb-t2mi)](https://docs.rs/dvb-t2mi)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#related-crates-and-license)

**DVB-T2 Modulator Interface (T2-MI, ETSI TS 102 773) packet and payload parser and builder, with a TS-in/typed-payload-out pump and one-call inner-TS recovery.**

## What it does / what it does not

T2-MI is the protocol between a DVB-T2 gateway and a DVB-T2 modulator. Each
T2-MI packet is a 6-byte header, a variable payload and a CRC-32, carried inside
MPEG-2 TS packets on one PID. The payloads carry Baseband Frames (the user
data), L1 signalling, auxiliary I/Q cells, DVB-T2 timestamps for SFN
synchronisation, individual-addressing commands and future-extension-frame
(FEF) parts.

This crate parses every one of the 12 packet types of TS 102 773 Table 1 into a
typed struct, serializes them back byte-exactly, validates the CRC, reassembles
packets from TS, and can chain through `dvb-bbframe` to recover the inner
transport stream. It is used by gateway/modulator test tools and stream
analyzers (`dvb-tools t2mi` is the complete CLI wrapper).

It does not modulate, decode L1 into physical-layer parameters beyond the typed
fields, interpret the inner stream's SI (see `dvb-si`), or decode HEM frames with
`MATYPE.NPD` (Null-Packet-Deletion DNP reinsertion is not modelled: such frames
are skipped, never mis-decoded).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI TS 102 773, *DVB-T2 Modulator Interface (T2-MI)* | V1.4.1 (vendored; verified) | §5.1 packet header; §5.2.1 to §5.2.12 all 12 packet types (BBFrame 0x00, aux I/Q 0x01, arbitrary cells 0x02, L1-current 0x10, L1-future 0x11, P2 bias 0x12, timestamp 0x20, individual addressing 0x21, FEF null 0x30, FEF I/Q 0x31, FEF composite 0x32, FEF sub-part 0x33); Annex A CRC-32 | Full | Unknown/reserved `packet_type` values fall through to `AnyPayload::Unknown` with bytes preserved. |
| ETSI TS 101 191, *DVB mega-frame for SFN synchronization* | V1.4.1 (vendored; verified) | §6.1 individual addressing functions 0x00-0x06 (time offset, frequency offset, power, private data, cell id, enable, bandwidth), the function bodies TS 102 773 §5.2.8.1 defers to | Full (Tables 7-13); the MIP/mega-frame parts are out of scope | `function_length` counts the whole `function()` incl. its 2-byte header (TS 102 773 §5.2.8.2.0, confirmed on a real capture); `ch_bandwidth` is 7 bits per Table 13 despite "8-bit" prose. |
| ETSI EN 302 755, *DVB-T2 frame structure, channel coding and modulation* | V1.4.1 (vendored; verified) | §7.2 L1-pre and L1-post signalling structures and their coded-value enums | Full for L1-pre/L1-post parsing | Used by `L1CurrentPayload`/`L1FuturePayload::l1_pre()` / `l1_post()`. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std`; off gives `no_std` + `alloc` |
| `ts` | yes | `pump::T2miPump` (PID-filtered TS reassembly + CRC validation), `ts::PacketReassembler`, `inner_ts::InnerTsRecovery`; pulls in `bytes`, `mpeg-ts`, `dvb-bbframe` |
| `serde` | no | `serde::Serialize` on every header/payload type (serialize-only; re-parse from wire bytes) |
| `chrono` | no | `T2TimestampPayload::utc_emission_time()` returning `DateTime<Utc>` |
| `yoke` | no | `yoke::Yokeable` on the zero-copy payload views, to own a parsed payload past the input borrow |

Every optional feature except `std` is `no_std` + `alloc` clean (built on
`thumbv7em` in CI).

```toml
[dependencies]
dvb-t2mi = "12"
```

## Quick start

Pump a real T2-MI capture (committed fixture, T2-MI on PID `0x0040`; the path is
relative to the crate root) and get typed payloads:

```rust
# #[cfg(feature = "ts")] {
use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::pump::T2miPump;
use mpeg_ts::pid::Pid;

let data = std::fs::read("../fixtures/dvb-t2mi/colombia-capital-t2mi.ts").unwrap();
let mut pump = T2miPump::new(Pid::try_new(0x0040).unwrap()); // T2-MI PID from the PMT
let (mut bbframes, mut l1) = (0u32, 0u32);
for packet in data.chunks_exact(188) {
    for event in pump.feed_ts(packet) {            // CRC-valid packets only
        match event.payload() {
            Ok(AnyPayload::Bbframe(bb)) => { let _ = bb.plp_id; bbframes += 1 }
            Ok(AnyPayload::L1Current(_)) => l1 += 1,  // l1.l1_pre(), l1.l1_post()
            _ => {}
        }
    }
}
assert!(bbframes > 0 && l1 > 0);
assert_eq!(pump.stats().crc_failures, 0);
# }
```

For an un-encapsulated `.t2mi` byte stream use `T2miPump::raw()` and
`feed_raw`. Recover the inner TS in one call (BBFrame extraction, NM/HEM,
carry-over) with `inner_ts::InnerTsRecovery`:

```rust
# #[cfg(feature = "ts")] {
use dvb_t2mi::inner_ts::InnerTsRecovery;
use mpeg_ts::pid::Pid;

let data = std::fs::read("../fixtures/dvb-t2mi/colombia-capital-t2mi.ts").unwrap();
let mut rec = InnerTsRecovery::new(Pid::try_new(0x0040).unwrap());
let mut inner = 0usize;
for outer in data.chunks_exact(188) {
    for pkt in rec.feed(outer) {      // recovered 188-byte inner TS packets
        assert_eq!(pkt[0], 0x47);
        inner += 1;
    }
}
assert!(inner > 0);
# }
```

`InnerTsRecovery::new_for_plp(pid, plp_id)` restricts recovery to one PLP;
`plps_seen()` lists every PLP a BBFrame was seen for (with more than one PLP the
unfiltered output interleaves independent TSes).

CLI: `cargo run -p dvb-tools -- t2mi file.ts [--pid 0xNNN|raw] [--inner] [--plp N]`.
Examples: `cargo run -p dvb-t2mi --example parse_header` and `--example pump_capture`.

## What is implemented

| Area | Public API |
|---|---|
| Packet framing | `packet::{Header, PacketType}` (`PacketType::is_allocated`), `crc::{validate_crc, CRC_LEN}`, `error::{Error, Result}` |
| Payloads (`payload`) | `BbframePayload`, `AuxIqPayload`, `ArbitraryCellsPayload`, `L1CurrentPayload`, `L1FuturePayload`, `P2BiasPayload`, `T2TimestampPayload` (+ `Bandwidth`), `IndividualAddressingPayload` (+ addressing functions), `FefNullPayload` (+ `S1Field`), `FefIqPayload`, `FefCompositePayload`, `FefSubPartPayload` (+ `SubpartVariety`, `PrbsType`); `FrequencySource` |
| Dispatch | `payload::AnyPayload` (generated from one `declare_payloads!` list with a drift test pinning each `packet_type` to its `PayloadDef::PACKET_TYPE`), `traits::PayloadDef` |
| L1 signalling | `L1Pre`, `L1Post` (`L1PostConfigurable`, `L1PostDynamic`, `PlpConfig`, `PlpDynamic`, `FefInfo`, `AuxConfig`, `AuxDynamic`, `L1ExtBlock`) and the coded-value enums `GuardInterval`, `L1Modulation`, `L1CodeRate`, `L1FecType`, `PilotPattern`, `PaprReduction`, `PlpType`, `PlpPayloadType`, `PlpModulation`, `PlpCodeRate`, `PlpFecType`, `PlpMode`, `AuxStreamType`, `TxInputStreamType`, `T2Version`, `RfFrequency` |
| Timestamp | `T2TimestampPayload::emission_offset()` (`Option<Duration>`, per-bandwidth `T_sub` units, Table 4), `is_null()`, `is_relative()`; `utc_emission_time()` with `chrono` |
| Private types | `payload::{PayloadRegistry, PayloadObject}`; `T2miEvent::payload_with(&registry)` yields `AnyPayload::Other` carrying your type for downcast |
| TS front door (`ts`) | `pump::{T2miPump, T2miEvent, Stats}` (`new`, `raw`, `feed_ts`, `feed_raw`, `stats`), `ts::PacketReassembler`, `inner_ts::InnerTsRecovery` |

## Design notes

- **Zero-copy, symmetric.** Payload views borrow the input; each has `Parse` +
  `Serialize` and round-trip tests; `tests/chain.rs` runs a synthetic T2-MI ->
  BBFrame -> inner TS -> SI chain.
- **RFU policy (stricter than the workspace default, documented):** payload
  parsers reject non-zero reserved (RFU) bits with `ReservedBitsViolation` and
  serialize them as 0. The one exception is individual addressing (0x21), which
  preserves its leading RFU byte verbatim so gateway streams round-trip
  byte-exactly.
- **Individual addressing framing.** `function_length` counts the whole
  `function()` including its 2-byte tag+length header (TS 102 773 §5.2.8.2.0),
  and a value below 2 is rejected; functions 0x00-0x06 (TS 101 191 §6.1) are typed.
- **Trait-driven dispatch.** Adding a packet type is one line in
  `declare_payloads!`; the completeness test walks the generated set.
- **Label convention.** Public enums carry `name()` and `Display`.
- `#![warn(missing_docs)]`, `no_std` + `alloc`.

## Related crates and license

Depends on `broadcast-common`, and (with `ts`) on `mpeg-ts` and `dvb-bbframe`.
The full chain: T2-MI here, BBFrame extraction in `dvb-bbframe`, SI in `dvb-si`.
Wrapped by the `dvb-tools t2mi` subcommand. Lockstep version with `dvb-si`,
`dvb-bbframe`, `dvb-conformance` and `dvb-tools`.
Spec transcriptions: [`docs/`](https://github.com/fishloa/rust-broadcast/tree/main/dvb-t2mi/docs).

MSRV 1.95.0. Licensed under [MIT](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-MIT) or
[Apache-2.0](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-APACHE), at your option.
