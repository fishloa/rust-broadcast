# rmt-flute

[![Crates.io](https://img.shields.io/crates/v/rmt-flute.svg)](https://crates.io/crates/rmt-flute)
[![docs.rs](https://img.shields.io/docsrs/rmt-flute)](https://docs.rs/rmt-flute)

Multicast object-delivery wire formats — **ALC / LCT / FLUTE / NORM** — the
binary headers used to deliver files and streams over IP multicast.

Everything here is **IETF RMT** (Reliable Multicast Transport): RFC 5651,
RFC 5775, RFC 6726 and RFC 5740. No broadcast-specific standard is implemented
by this crate. Several delivery systems are built *on top* of these formats and
are consumers of it, not owners of it:

- **DVB** — DVB-IPTV and DVB-MABR (ETSI TS 103 769) file delivery
- **3GPP** — MBMS / eMBMS download delivery
- **ATSC 3.0** — ROUTE (A/331 Annex A), written as a profile-and-delta on
  RFC 5651/5775/6726

> **Renamed from `dvb-flute` at 0.4.0.** The old name implied a DVB standard
> this crate does not implement, and made it read oddly as a dependency of
> non-DVB consumers such as ATSC 3.0 ROUTE. All `dvb-flute` versions are
> yanked; there is no shim. Same code, accurate name.

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
use rmt_flute::{LctHeader, LCT_VERSION};

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
cargo run -p rmt-flute --example build_lct
cargo run -p rmt-flute --example parse_flute
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
