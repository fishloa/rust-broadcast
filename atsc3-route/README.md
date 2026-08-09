# atsc3-route

[![Crates.io](https://img.shields.io/crates/v/atsc3-route.svg)](https://crates.io/crates/atsc3-route)
[![docs.rs](https://img.shields.io/docsrs/atsc3-route)](https://docs.rs/atsc3-route)

ATSC A/331 Annex A **ROUTE** (Real-time Object delivery over Unidirectional
Transport) binary framing — split out of the `atsc3` crate (issue #943) so a
`no_std` ROUTE receiver doesn't have to pull in an XML stack. `atsc3` keeps
the signalling half (LLS + SLS XML, needs `roxmltree` + `flate2`); this crate
is the binary ROUTE delta, built directly on [`rmt-flute`](../rmt-flute)'s
LCT/ALC/FLUTE implementation of the RFC 5651/5775/6726 base A/331 Annex A
profiles.

## Install

```toml
[dependencies]
atsc3-route = "0.1"
```

## What this implements

A/331 Annex A is written as a profile-and-delta on RFC 5651 (LCT), RFC 5775
(ALC) and RFC 6726 (FLUTE) — this crate implements exactly the delta, not the
base RFCs (that's `rmt-flute`'s job). The full transcription lives at
`atsc3/docs/a331-route.md` (a sibling crate's `docs/`, since the doc predates
this crate's split and documents both halves of A/331 Annex A).

- **[`RoutePacket`]** — a composed ROUTE ALC/LCT packet: an
  `rmt_flute::LctHeader` constrained to A/331's mandated field widths
  (§A.3.4/§A.3.6: `V`=1, `C`=00, `S`=1, `O`=01, `H`=0, and the PSI/SPI rule),
  the SPI-bit-dispatched `RouteFecPayloadId`, and the opaque delivery-object
  payload.
- **`EXT_ROUTE_PRESENTATION_TIME`** (HET 66, §A.3.7.1) — `ExtRoutePresentationTime`:
  the full 64-bit NTP presentation time of an MDE Random Access Point.
- **`EXT_TOL`** (HET 194 fixed-length / 67 variable-length, §A.3.8.1) —
  `ExtTol::Bits24`/`Bits48`: the delivery object's post-content-encoding
  transfer length.
- **FEC Payload ID layouts** (§A.3.5.1/§A.3.5.2) — `SourceFecPayloadId`
  (Compact No-Code `start_offset`, source flows) and `RepairFecPayloadId`
  (RaptorQ `SBN`/`ESI` per RFC 6330 §3.2, repair flows), dispatched by the
  LCT SPI bit via `RouteFecPayloadId`.
- **Codepoint (`CP`) semantics** (§A.3.6, Table A.3.6) — `Codepoint`,
  `FormatId` (Table A.3.2), `FragMode`, and `Codepoint::known_semantics()`
  for the directly-resolvable `CP` values 1-9.

## Scope

Out of scope, same as `rmt-flute`: the FDT/S-TSID/USBD/MPD **XML** documents
carried as opaque ROUTE payload bytes (the `atsc3` crate's job, or a
consumer's own XML layer), and the RaptorQ encode/decode procedure itself
(RFC 6330 is not vendored in this repository).

## ⚠ Repair-flow coverage gap

The real ROUTE capture fixtures this crate is verified against
(`fixtures/atsc3/route-*.bin`) come from a session that ran with the LCT SPI
bit set on **every** packet across both source `.pcap` files (8,885 frames
scanned) — no FEC-repair flow was ever active in that capture.
`RepairFecPayloadId` is implemented directly from A/331's own Figure A.3.4
bit-diagram (matching RFC 6330 §3.2 exactly) and unit-tested against
hand-built vectors, but has **no real-capture corroboration**. See
`fixtures/atsc3/PROVENANCE.md`'s "What was not obtained" section.

`#![no_std]` + `alloc` (via `rmt-flute`); depends only on `rmt-flute` and
`broadcast-common`.

## Quick start

```rust
use atsc3_route::{Codepoint, RoutePacket};
use broadcast_common::{Parse, Serialize};

let cci = [0u8; 4];
let tsi = 3000u32.to_be_bytes();
let toi = 6034u32.to_be_bytes();
let lct = rmt_flute::LctHeader {
    version: rmt_flute::LCT_VERSION,
    psi: rmt_flute::PSI_SPI,
    close_session: false,
    close_object: false,
    codepoint: 128,
    cci: &cci,
    tsi: &tsi,
    toi: &toi,
    extensions: vec![],
};
let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
let pkt = RoutePacket {
    lct,
    fec_payload_id: atsc3_route::RouteFecPayloadId::Source(
        atsc3_route::SourceFecPayloadId { start_offset: 1408 },
    ),
    payload: &payload,
};

let bytes = pkt.to_bytes();
let re = RoutePacket::parse(&bytes).unwrap();
assert_eq!(re, pkt);
assert!(matches!(re.codepoint(), Codepoint::Indirect(128)));
```

## Examples

```sh
cargo run -p atsc3-route --example parse_route_fdt
cargo run -p atsc3-route --example build_route_media_fragment
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
