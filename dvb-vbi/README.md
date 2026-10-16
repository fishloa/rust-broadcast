# dvb-vbi

[![Crates.io](https://img.shields.io/crates/v/dvb-vbi.svg)](https://crates.io/crates/dvb-vbi)
[![docs.rs](https://img.shields.io/docsrs/dvb-vbi)](https://docs.rs/dvb-vbi)

VBI (Vertical Blanking Interval) data carriage in DVB: parser and serializer for the
EN 301 775 PES data field, covering VPS, WSS, Closed Captioning, EBU/Inverted
Teletext and monochrome 4:2:2 luminance sample data units.

## What it does / what it does not

**Does.** VBI data (analogue lines carrying teletext, widescreen signalling,
line-21 captions, VPS) is carried in a DVB PES packet with
`stream_id = private_stream_1` (`0xBD`). This crate decodes the data field that
follows the PES header into typed data units and serializes it back
byte-for-byte. Users are stream analysers, re-multiplexers and anything
regenerating VBI services from a DVB stream.

**Does not.**
- It does not decode Teletext pages: the 42-byte `txt_data_block` is carried as
  an opaque block (EN 300 706 coding is out of scope).
- It does not decode CEA-608 characters from the 16-bit closed-caption block, or
  interpret WSS aspect-ratio codes or VPS fields (see `cc-data` and
  `caption-convert` for captions).
- It does not re-insert VBI into analogue video, and does not parse the PES
  header or TS (use `mpeg-ts` and `mpeg-pes`) or the PMT `VBI_data_descriptor`
  (`dvb-si`).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI EN 301 775, Carriage of Vertical Blanking Information (VBI) data in DVB bitstreams | V1.2.1 (transcription `etsi_en_301_775_v01.02.01_vbi.md`, listed in `private/specs-md/INDEX.md`) | §4.4.1 PES data field (Table 1), `data_identifier` (Table 2), `data_unit_id` (Table 3), stuffing | Full | `data_unit_length` recomputed on serialize; trailing `stuffing_byte`s kept as a typed count |
| | | §4.5 EBU Teletext (`0x02`, `0x03`) and Inverted Teletext (`0xC0`) | Typed-opaque | Line header and framing code typed; 42-byte block opaque |
| | | §4.6 VPS (`0xC3`), §4.7 WSS (`0xC4`) | Full | 13-byte VPS block; 14-bit WSS block plus reserved tail |
| | | §4.8 Closed Captioning (`0xC5`) | Typed-opaque | 16-bit block, not decoded into 608 characters |
| | | §4.9 Monochrome 4:2:2 samples (`0xC6`) | Full | Segment flags, parity, offset, pixel position, Y values |
| | | Reserved / user-defined `data_unit_id` | Typed-opaque | `Opaque` payload, round-trips verbatim |
| ETSI EN 300 472 (EBU Teletext in DVB), EN 300 231 (VPS), EN 300 294 (WSS), EIA-608 | referenced by EN 301 775 | Underlying analogue-service codings | Not implemented | Only the DVB carriage is parsed |

Spec ambiguity: Table 1's parse branch routes `data_unit_id` `0xC1` to
`txt_data_field()`, but Table 3 marks `0xC1` as reserved (discard). This crate
follows Table 3, so `0xC1` decodes as `DataUnitId::Reserved`.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std`. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no | `serde::Serialize` derives on public types. |

## Quick start

```rust
use dvb_vbi::{DataField, DataUnit, LineHeader, VpsDataField, WssDataField};

let vps = DataUnit::vps(VpsDataField {
    header: LineHeader::new(true, 16),
    vps_data_block: [0u8; 13],
});
let wss = DataUnit::wss(WssDataField::new(LineHeader::new(true, 23), 0x1234));
let field = DataField::new(0x10, vec![vps, wss]);

let mut buf = vec![0u8; field.serialized_len()];
field.serialize_into(&mut buf).unwrap();
assert_eq!(DataField::parse(&buf).unwrap(), field);
```

Runnable examples: `cargo run -p dvb-vbi --example build_data_field` and
`--example parse_data_field`.

## What is implemented

- `DataField`: `data_identifier` plus a loop of `DataUnit`s.
- `DataUnit` / `DataUnitId` / `DataUnitPayload`: id (Table 3), length, typed
  body, trailing stuffing count, with convenience constructors
  (`DataUnit::vps`, `::wss`, `::teletext`, ...).
- `LineHeader`: the shared first byte (2-bit reserved prefix, field parity,
  5-bit line offset).
- `TeletextDataField`, `VpsDataField`, `WssDataField`,
  `ClosedCaptioningDataField`, `MonochromeDataField`.
- `Error` (`#[non_exhaustive]`): `BufferTooShort`, `OutputBufferTooSmall`,
  `InvalidDataUnitLength`, `FieldTooWide`, `InvalidField`.

## Design notes

- Symmetric `Parse` / `Serialize`; parse then serialize is byte-identical,
  tested on a committed capture (`dvb-vbi/vbi_data_field.bin` via `test-fixtures`).
- No raw passthrough: reserved bits and stuffing are typed fields, so
  non-conformant input still round-trips; `Opaque`/`Stuffing` units that own
  their whole body reject a non-zero extra stuffing count on serialize.
- Hostile input never panics (`tests/hostility.rs`).
- Public enums are `#[non_exhaustive]` with `name()` and `Display`.
- `no_std` + `alloc`.

## Related crates

Depends only on `broadcast-common`. Typical pipeline: `mpeg-ts` -> `mpeg-pes`
-> `dvb-vbi`. `dvb-si` carries the PMT descriptors that signal VBI streams;
`cc-data` and `caption-convert` handle caption content.

## MSRV and licence

Rust 1.95.0. MIT OR Apache-2.0.
