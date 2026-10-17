# mpeg-ts

[![crates.io](https://img.shields.io/crates/v/mpeg-ts.svg)](https://crates.io/crates/mpeg-ts)
[![docs.rs](https://img.shields.io/docsrs/mpeg-ts)](https://docs.rs/mpeg-ts)
[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)

**MPEG-2 Transport Stream framing: TS packets, adaptation fields, PCR, PSI section reassembly and packetisation, byte-stream resync.**

## What it does / what it does not

`mpeg-ts` is the lowest layer of the workspace's broadcast stack. It turns a
stream of bytes into typed 188-byte transport packets (and back), and it turns
the payload of PSI/SI PIDs into complete sections (and back). It is for anyone
who needs raw TS framing without the SI/descriptor stack: demuxers, muxers,
stream repair tools, analyzers, embedded firmware.

It does:

- parse and serialize the 4-byte TS header and the adaptation field (every
  bit, including stuffing and reserved bits, so parse -> serialize is
  byte-exact on real streams);
- reassemble PSI/SI sections from TS payloads (`pointer_field`, multi-packet,
  several sections per packet) and packetise sections back, with continuity
  counters and `0xFF` stuffing;
- schedule repeating SI tables into a TS packet stream (`SiMux`);
- recover packet alignment from a raw byte stream (188- and 204-byte strides);
- accumulate PUSI-delimited payloads on non-PSI PIDs (e.g. DASH `emsg`);
- provide an owned, mutable packet type for in-place edits (continuity counter,
  PCR).

It deliberately does not: parse PES (see `mpeg-pes`), interpret PSI/SI table
bodies or descriptors (see `dvb-si`), descramble (see `dvb-csa`), check
stream conformance (see `dvb-conformance`), or do any I/O.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1, *Generic coding of moving pictures and associated audio information: Systems* | H.222.0 v9 (08/2023); vendored copy verified in `private/specs-md` | §2.4.3.2 TS packet layer, §2.4.3.3 field semantics, §2.4.3.4 adaptation field (incl. PCR/OPCR, splice, LTW, piecewise rate, extension), §2.4.4 PSI `pointer_field` and section framing | Full for the listed clauses | Other H.222.0 clauses (PES §2.4.3.6, program streams §2.5, descriptors) are not this crate's job. Layouts cited by clause in module docs; 2007-era layouts for these clauses are unchanged. |
| ETSI EN 300 468 | V1.19.1 (the edition vendored; a V1.20.1 copy is also present) | §5.1.1 generic section header, §5.1.3 Table 1 well-known PIDs, CRC_32 placement | Partial: framing and PID constants only | Table bodies and descriptors live in `dvb-si`. The TOT's CRC is not handled by the short-form path (use `dvb-si`). |
| ISO/IEC 23009-1 | not pinned by this crate | §5.10.3.3.5 PUSI-delimited `emsg` carriage on PID `0x0004` | Partial: generic PUSI reassembly only | The `emsg` box itself is parsed by `mp4-emsg`. |
| 204-byte RS-coded packets (188 + 16 parity) | n/a | Stride detection in `TsResync` | Partial | Parity bytes are stripped, never checked. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | `std::error::Error` impls; turn off for `no_std` |
| `serde` | yes | `serde::Serialize` on packet/section types (serialize-only) |

The crate is `#![no_std]` + `alloc` and `#![forbid(unsafe_code)]`;
`default-features = false` builds on bare-metal targets that have a heap.

```toml
[dependencies]
mpeg-ts = "0.6"
# embedded:
# mpeg-ts = { version = "0.5", default-features = false }
```

## Quick start

Parse a real PAT packet (extracted from the committed `m6-single.ts` fixture)
and recover its section:

```rust
use mpeg_ts::ts::{SectionReassembler, TsPacket};
use mpeg_ts::pid::well_known;

// PID 0x0000, PUSI set, payload-only; PAT section: transport_stream_id 1, one program.
let mut raw = [0xFFu8; 188];
raw[..21].copy_from_slice(&[
    0x47, 0x40, 0x00, 0x10, // sync, PUSI + PID 0, payload only, CC 0
    0x00,                   // pointer_field
    0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x04, 0x01, 0xE0, 0x64, // PAT
    0xF9, 0xB4, 0x63, 0xEF, // CRC_32
]);

let pkt = TsPacket::parse(&raw)?;
assert_eq!(pkt.header.pid, well_known::PAT);
assert!(pkt.header.pusi);

let mut reasm = SectionReassembler::default();
reasm.feed(pkt.payload.expect("payload"), pkt.header.pusi);
let section = reasm.pop_section().expect("one complete section");
assert_eq!(section[0], 0x00); // table_id = PAT
assert_eq!(u16::from_be_bytes([section[3], section[4]]), 1); // transport_stream_id
# Ok::<(), mpeg_ts::Error>(())
```

Build and edit packets without manual byte twiddling:

```rust
use mpeg_ts::ts::{Pcr, TsPacket};
use mpeg_ts::{pid::{well_known, Pid}, OwnedTsPacket};

let null = OwnedTsPacket::null_packet(3);
assert_eq!(TsPacket::parse(&null)?.header.pid, well_known::NULL);

let mut raw = OwnedTsPacket::serialize_with_payload(Pid::try_new(0x0100).unwrap(), false, 0, &[])?;
OwnedTsPacket::set_continuity_counter(&mut raw, 7);
assert_eq!(TsPacket::parse(&raw)?.header.continuity_counter, 7);

let mut pcr_pkt = [0x47u8; 188];
pcr_pkt[3] = 0x20; // adaptation only
pcr_pkt[4] = 7;    // adaptation_field_length
pcr_pkt[5] = 0x10; // PCR flag
OwnedTsPacket::set_pcr(&mut pcr_pkt, Pcr::from_27mhz(27_000_000))?;
let parsed = TsPacket::parse(&pcr_pkt)?;
assert_eq!(parsed.adaptation_field().unwrap()?.pcr.unwrap().as_27mhz(), 27_000_000);
# Ok::<(), mpeg_ts::Error>(())
```

Runnable examples: `cargo run -p mpeg-ts --example demux`, `--example iter_packets`,
`--example edit_packet`.

## What is implemented

| Module | Public API | Purpose |
|---|---|---|
| `ts` | `TsPacket<'a>`, `TsHeader`, `AdaptationField<'a>`, `AdaptationFieldExtension<'a>`, `Pcr`, `Ltw`, `SeamlessSplice`, `ScramblingControl`, `AdaptationFieldControl`, `SectionReassembler`, `iter_packets`, `extract_ts_payload`, `TS_PACKET_SIZE` | Zero-copy packet view, bit-exact adaptation field, PCR (27 MHz) and per-PID PSI section reassembly |
| `owned` | `OwnedTsPacket` | Owned 188-byte packet: build (`null_packet`, `serialize_with_payload`), mutate (`set_continuity_counter`, `set_pcr`, `payload_mut`), inspect |
| `pid` | `Pid`, `PidOutOfRange`, `well_known::{PAT, CAT, TSDT, IPMP_CIT, NIT, SDT_BAT, EIT, RST, TDT_TOT, NETWORK_SYNC, RNT, SAT, INBAND_SIGNALLING, MEASUREMENT, DIT, SIT, ATSC_PSIP, NULL}` | Typed 13-bit PID newtype |
| `section` | `Section<'a>` | Generic section header (`table_id`, syntax indicator, length, version, section numbers, CRC validation) |
| `mux` | `SectionPacketiser`, `SiMux` (`upsert`, `upsert_pat`, `upsert_pmt`, `upsert_sdt_actual`, `upsert_nit`, `upsert_tdt`, `upsert_tot`, `poll`, `poll_into`) | Sections to TS packets with continuity and stuffing; interval-scheduled SI repetition |
| `resync` | `TsResync`, `PacketStride` (`Ts188`, `Rs204`), `ResyncStats` | Alignment recovery from raw bytes; `feed_into` appends to a caller-owned vector |
| `pusi` | `PusiReassembler`, `DEFAULT_MAX_UNIT_SIZE` | PUSI-delimited unit accumulation for non-PSI PIDs, size-capped |
| `error` | `Error`, `Result` | Structured `thiserror` errors (`BufferTooShort`, `CrcMismatch`, `InvalidSyncByte`, `OutputBufferTooSmall`, `FieldOverflow`, ...) |

Typed PIDs: `Pid` is used everywhere (`TsHeader::pid`, the mux types,
`PusiReassembler`, `OwnedTsPacket`). `Pid::try_new` / `TryFrom<u16>` reject
values above `0x1FFF`; `Pid::new` / `Pid::from_wire_masked` mask bits already
taken from the wire; there is deliberately no `From<u16>`. `Pid` implements
`UpperHex`/`LowerHex` and is `serde(transparent)`.

## Design notes

- **Zero-copy, symmetric.** Parsed types borrow the input; every wire type has
  a matching serializer. `tests/fixture_roundtrip.rs` and
  `tests/af_extension_roundtrip.rs` re-serialize every adaptation field of
  every committed `.ts` fixture and require byte-identical output.
- **No information dropped.** Adaptation-field stuffing, `pcr_reserved` /
  `opcr_reserved`, splice marker bits and every extension reserved bit are
  carried; the structs are `#[non_exhaustive]` with `new()` / `Default`
  (reserved bits default to `1`, per the spec convention).
- **Checked widths.** Serializers use `broadcast_common::len` checked helpers
  (`FieldOverflow`), never a silent mask.
- **Shared clock arithmetic.** PCR and the marker-bit timestamp come from
  `broadcast_common::clock33`.
- **Label convention.** Public field enums expose `name()` and `Display`
  (`tests/label_coverage.rs` guards it).

## Related crates

Depends on `broadcast-common`. Built on by `dvb-si` (tables/descriptors),
`mpeg-pes`, `dvb-stream`, `dvb-conformance`, `ts-fix`, `media-doctor` and
`transmux`. Spec citations: [`docs/`](https://github.com/fishloa/rust-broadcast/tree/main/mpeg-ts/docs).
Users migrating from `dvb-si <= 7.x`: `ts`, `mux`, `resync` and `pid` moved here
unchanged in name.

MSRV 1.95.0. Licensed under [MIT](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-MIT) or
[Apache-2.0](https://github.com/fishloa/rust-broadcast/blob/main/LICENSE-APACHE), at your option.
