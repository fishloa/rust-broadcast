# dvb-vbi

[![Crates.io](https://img.shields.io/crates/v/dvb-vbi.svg)](https://crates.io/crates/dvb-vbi)
[![docs.rs](https://img.shields.io/docsrs/dvb-vbi)](https://docs.rs/dvb-vbi)

VBI (Vertical Blanking Information) data carriage in DVB — ETSI EN 301 775 §4,
the **PES data field**: VPS, WSS, Closed Captioning, EBU / Inverted Teletext,
and generic monochrome 4:2:2 luminance sample data units.

VBI data is carried in the private PES packet mechanism
(`stream_id = private_stream_1` `0xBD`); this crate decodes the data field that
follows the PES header.

Implements:

- **`DataField`** — the PES data field (§4.4.1, Table 1): a `data_identifier`
  byte (Table 2) followed by a loop of data units.
- **`DataUnit`** / **`DataUnitId`** — each data unit's `data_unit_id` (Table 3)
  + 8-bit `data_unit_length` + typed body. `data_unit_length` is recomputed from
  the typed body on serialize (no raw passthrough).
- **`TeletextDataField`** — EBU (`0x02`/`0x03`) and Inverted (`0xC0`) Teletext
  (§4.5): a shared `LineHeader` + an 8-bit `framing_code` + a 42-byte opaque
  `txt_data_block`. EN 300 706 Teletext coding is out of scope.
- **`VpsDataField`** — VPS (`0xC3`, §4.6): shared header + 13-byte data block.
- **`WssDataField`** — WSS (`0xC4`, §4.7): shared header + a 14-bit
  `wss_data_block` + a 2-bit `reserved_future_use` `11` tail (3 bytes total).
- **`ClosedCaptioningDataField`** — Closed Captioning (`0xC5`, §4.8): shared
  header + a 16-bit data block (line 21, EIA-608 Rev A).
- **`MonochromeDataField`** — monochrome 4:2:2 samples (`0xC6`, §4.9): its own
  first-byte packing (first/last segment flags + field_parity + line_offset),
  a `first_pixel_position`, `n_pixels`, and the luminance `Y_value` bytes.
- Stuffing (`0xFF`, §4.4.1) and an `Opaque` catch-all for reserved /
  user-defined ids round-trip verbatim.

The shared `LineHeader` is the Teletext/VPS/WSS/CC first byte
(reserved_future_use `11` | field_parity | 5-bit line_offset).

> ⚠ Table 1's parse branch routes `data_unit_id` `0xC1` to `txt_data_field()`,
> but Table 3 marks `0xC1` as *reserved → discard*. This crate follows Table 3
> (authoritative), so `0xC1` decodes as `DataUnitId::Reserved`.

`#![no_std]` + `alloc`; depends only on `dvb-common`.

## Quick start

```rust
use dvb_vbi::{DataField, DataUnit, LineHeader, VpsDataField, WssDataField};

let vps = DataUnit::vps(VpsDataField {
    header: LineHeader::new(true, 16),
    vps_data_block: [0u8; 13],
});
let wss = DataUnit::wss(WssDataField {
    header: LineHeader::new(true, 23),
    wss_data_block: 0x1234,
});
let field = DataField::new(0x10, vec![vps, wss]);

let mut buf = vec![0u8; field.serialized_len()];
field.serialize_into(&mut buf).unwrap();
assert_eq!(DataField::parse(&buf).unwrap(), field);
```

## Examples

```sh
cargo run -p dvb-vbi --example build_data_field
cargo run -p dvb-vbi --example parse_data_field
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
