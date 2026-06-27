# ule

[![Crates.io](https://img.shields.io/crates/v/ule.svg)](https://crates.io/crates/ule)
[![docs.rs](https://img.shields.io/docsrs/ule)](https://docs.rs/ule)

ULE — Unidirectional Lightweight Encapsulation (RFC 4326) with Extension
Headers (RFC 5163): IP (and other PDUs) over MPEG-2 Transport Streams.

Implements:

- **`Sndu`** — the SubNetwork Data Unit wire structure (RFC 4326 §4): the `D`
  bit + 15-bit `Length` + 16-bit `Type`, an optional 6-byte Destination NPA
  address (present iff `D = 0`), the PDU, and the 4-byte CRC-32 trailer.
  `Length` and the CRC are **recomputed on serialize** from the typed fields.
- **`TypeField`** — the §4.4 split at `0x0600`: a Next-Header (`H-LEN`/`H-Type`)
  below, an EtherType at or above.
- **`ExtensionHeader`** / **`PayloadChain`** — chained extension headers (RFC
  4326 §5, RFC 5163 §3): Optional headers (`H-LEN = 1..=5`, total `2·H-LEN`
  bytes) terminated by an EtherType or a Mandatory header (Test-SNDU `0x00`,
  Bridged-Frame `0x01`, TS-Concat `0x02`, PDU-Concat `0x03`).
- **`UleReceiver`** — TS-packet de-fragmentation/reassembly (RFC 4326 §6, §7):
  PUSI + 1-byte Payload Pointer handling, fragmentation across packets, packing
  of multiple SNDUs per packet, and End-Indicator / `0xFF` padding.

The CRC-32 is the MPEG-2 / DSM-CC CRC (poly `0x04C11DB7`, init `0xFFFFFFFF`,
MSB-first, no reflection, no final XOR — RFC 4326 §4.6), reused from
`dvb-common`. It is verified byte-exact against RFC 4326 Appendix B's worked
example (CRC `0x7C171763`) in the crate's fixture test.

`#![no_std]` + `alloc`; depends only on `dvb-common`.

## Quick start

```rust
use ule::{Sndu, TypeField};

let pdu = [0x45u8, 0x00, 0x00, 0x14]; // start of an IPv4 header
let sndu = Sndu::new(
    TypeField::EtherType(0x0800),
    Some([0x00, 0x01, 0x02, 0x03, 0x04, 0x05]),
    &pdu,
);
let mut buf = vec![0u8; sndu.serialized_len()];
sndu.serialize_into(&mut buf).unwrap();
assert_eq!(Sndu::parse(&buf).unwrap(), sndu);
```

## Examples

```sh
cargo run -p ule --example build_sndu
cargo run -p ule --example receive_sndu
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.81

## License

MIT OR Apache-2.0
