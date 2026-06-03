# dvb_bbframe

ETSI DVB-S2 / S2X / T2 Base-Band Frame (BBFRAME) parser and builder, supporting
both Normal Mode (NM) and High Efficiency Mode (HEM).

A BBFRAME is the unit of payload carried in a DVB-S2/S2X/T2 baseband stream: a
10-byte BBHEADER followed by a data field of user packets (a Transport Stream or
Generic Stream). This crate parses and rebuilds the header and walks the data
field; it does **not** implement the physical layer (LDPC/BCH coding, modulation).

## Coverage

| Module | Purpose |
|---|---|
| `header` | `Bbheader` — the 10-byte BBHEADER (MATYPE-1/2, UPL, DFL, SYNC, SYNCD, CRC-8), parse + serialize. Decodes `Mode` (NM/HEM), `TsGs`, and the MATYPE flags (SIS/MIS, CCM/ACM, ISSYI, NPD, ext, ISI). |
| `packet` | `up_iter` — user-packet extraction from the data field, honouring SYNCD and the UPL stride. |
| `crc` | `crc8` — CRC-8 per EN 302 307-1 §5.1.4 / EN 302 755 Annex F. The NM/HEM mode is recovered from `computed_crc8 ^ stored_byte`. |
| `issy` | ISSY (Input Stream Synchronizer) field parser, EN 302 755 Annex C. |

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `serde` | off | `Serialize`/`Deserialize` derives on `Bbheader`, `Matype`, `TsGs`, `Mode`, `Issy` |

`no_std` is not supported (`thiserror` requires `std`).

## Principles

- **Spec fidelity** — every BBHEADER field is exposed; bit positions match EN 302 307-1 / EN 302 755.
- **Parse and construct** — `Bbheader` round-trips: parse → serialize → parse is identity (CRC included).
- **No magic numbers** — every hex literal outside tests is a named constant or enum.

## Usage

```rust
use dvb_bbframe::header::{Bbheader, Mode};

let hdr = Bbheader::parse(frame)?;
assert_eq!(hdr.mode, Mode::Normal);
println!("UPL={} DFL={} SYNC=0x{:02X}", hdr.upl, hdr.dfl, hdr.sync);
```

## Authoritative references

- ETSI EN 302 307-1 (DVB-S2) / EN 302 307-2 (DVB-S2X)
- ETSI EN 302 755 (DVB-T2) — §5.1.7 (HEM), Annex C (ISSY), Annex F (CRC-8)

The structured spec reference is under [`docs/`](docs/); the canonical PDFs are
vendored in the workspace `specs/` directory.

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
