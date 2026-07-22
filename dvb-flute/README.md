# dvb-flute

[![Crates.io](https://img.shields.io/crates/v/dvb-flute.svg)](https://crates.io/crates/dvb-flute)
[![docs.rs](https://img.shields.io/docsrs/dvb-flute)](https://docs.rs/dvb-flute)

Multicast object-delivery wire formats — **ALC / LCT / FLUTE / NORM** — the
binary headers used to deliver files and streams over IP multicast (the
building blocks beneath DVB-IPTV / DVB-MABR file delivery and the IETF RMT
suite).

Implements:

- **`LctHeader`** — the Layered Coding Transport header (RFC 5651 §5). The fixed
  first word carries `V`/`C`/`PSI`/`S`/`O`/`H`/`A`/`B`, `HDR_LEN` and the
  Codepoint; the `C`, `S`, `O` and `H` flags then drive the byte-widths of the
  **CCI** (`4*(C+1)`), **TSI** (`4*S+2*H`) and **TOI** (`4*O+2*H`) fields. The
  shared `H` half-word feeds **both** TSI and TOI. Flag bits and `HDR_LEN` are
  recomputed on serialize from the typed field lengths — no raw passthrough.
- **`HeaderExtension`** — the LCT/NORM header-extension chain (RFC 5651 §5.2):
  variable-length (`HET` 0..=127, carries `HEL`) and fixed-length (`HET`
  128..=255, one word) forms; with `ExtTime` (EXT_TIME) and the `LctExtType`
  registry (EXT_NOP/EXT_AUTH/EXT_TIME).
- **`AlcPacket`** — an Asynchronous Layered Coding packet (RFC 5775): LCT header
  + an opaque FEC Payload ID + the encoding-symbol payload, plus `EXT_FTI`
  (HET 64) and the Small-Block-Systematic `FecPayloadId128`.
- **`ExtFdt` / `ExtCenc`** — the FLUTE (RFC 6726) fixed-length LCT extensions
  `EXT_FDT` (HET 192) and `EXT_CENC` (HET 193), plus the TOI = 0 FDT-Instance
  convention. The FDT Instance body is **XML and out of scope** of this binary
  crate — it rides as the packet payload.
- **`NormCommonHeader`** + **`NormData` / `NormCmd` / `NormFeedback`** — the NORM
  (RFC 5740) common header and message types (NORM_DATA / INFO / CMD / NACK /
  ACK / REPORT).

> ⚠ **FEC Payload ID** bit layouts are FEC-scheme dependent (RFC 5052 / the FEC
> Scheme document) and are **not** defined by ALC/NORM themselves; this crate
> exposes them as opaque byte slices (the caller supplies the length), with
> `FecPayloadId128` provided as one concrete illustrative layout.

`#![no_std]` + `alloc`; depends only on `broadcast-common`.

## Quick start

```rust
use dvb_flute::{LctHeader, LCT_VERSION};

let cci = [0u8; 4]; // C = 0
let tsi = [0u8; 4]; // S = 1, H = 0
let hdr = LctHeader {
    version: LCT_VERSION,
    psi: 0,
    close_session: false,
    close_object: false,
    codepoint: 0,
    cci: &cci,
    tsi: &tsi,
    toi: &[],
    extensions: vec![],
};
let mut buf = vec![0u8; hdr.serialized_len()];
hdr.serialize_into(&mut buf).unwrap();
let (re, used) = LctHeader::parse(&buf).unwrap();
assert_eq!(used, buf.len());
assert_eq!(re, hdr);
```

## Examples

```sh
cargo run -p dvb-flute --example build_lct
cargo run -p dvb-flute --example parse_flute
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize` derives on public types. |

## Minimum Supported Rust Version

1.86

## License

MIT OR Apache-2.0
