# dvb_si

ETSI EN 300 468 DVB Service Information parser and builder, with the MPEG-2 PSI
tables it builds on. Every table has a symmetric `Parse` / `Serialize`
implementation and is round-trip tested.

## Coverage

**Framing**
- Long- and short-form section header (§5.1.1) with CRC-32 validation
- `TsPacket` + `SectionReassembler` (feature `ts`)

**MPEG-2 PSI tables**
- PAT (0x00), CAT (0x01), PMT (0x02), TSDT (0x03)

**DVB SI tables (EN 300 468)**
- NIT actual/other (0x40/0x41), SDT actual/other (0x42/0x46), BAT (0x4A)
- EIT p/f + schedule actual/other (0x4E–0x6F); chrono-gated `start_time()` decodes MJD+BCD
- TDT (0x70), RST (0x71), ST (0x72), TOT (0x73), DIT (0x7E), SIT (0x7F)
- **SAT — Satellite Access Table family (0x4D, §5.2.11)**: typed section header +
  `SatTableId` discriminant (position v2/v3, cell fragment, time association,
  beamhopping time plan); bodies exposed raw, layout documented in `docs/en_300_468.md`

**Companion-spec tables (share the DVB table_id space)**
- AIT (0x74, TS 102 809), DSM-CC (0x3A–0x3F)
- **UNT** (0x4B, TS 102 006 — System Software Update)
- **INT** (0x4C, EN 301 192 — IP/MAC Notification)
- **RCT** (0x76), **CIT** (0x77), **RNT** (0x79) — TS 102 323 TV-Anytime

**Descriptors** (parsed into typed structs; others pass through as raw bytes)
- 0x09 CA, 0x0A ISO-639 language, 0x40 network_name, 0x41 service_list,
  0x43 satellite_delivery_system, 0x44 cable_delivery_system, 0x48 service,
  0x4A linkage, 0x4D short_event, 0x4E extended_event, 0x50 component,
  0x54 content, 0x55 parental_rating, 0x58 local_time_offset, 0x59 subtitling,
  0x5A terrestrial_delivery_system, 0x06 data_stream_alignment, 0x73 default_authority,
  0x76 content_identifier, 0x79 S2_satellite_delivery_system, 0x6A AC-3,
  0x7A Enhanced AC-3, plus bouquet_name, logical_channel, private_data_indicator,
  registration, stream_identifier, teletext, frequency_list.

**Text** — Annex A: ISO 6937 (with diacritic combining), ISO 8859-n (via
`encoding_rs`), UTF-8 (selector 0x15), UCS-2 BE (0x11); Annex A.2 control codes.

**CRC** — Annex C MPEG-2 CRC-32, compile-time table (from `dvb_common`).

> Tables expose their outer structure with typed fields; descriptor loops and
> other variable-length / repeated sub-structures are borrowed as raw `&[u8]`
> slices, which the caller walks with the descriptor parsers.

## Principles

- **Spec fidelity.** Every field in a section's syntax appears in the parsed struct.
- **No magic numbers.** Every hex literal outside `#[cfg(test)]` is a named constant or enum.
- **Zero-copy where possible.** Parsed types borrow from the input via `<'a>` lifetimes.
- **Parse and construct.** Every parser has a symmetric serializer; round-trip is tested.

## Usage

```rust
use dvb_common::Parse;
use dvb_si::tables::sdt::Sdt;

let sdt = Sdt::parse(&section_bytes)?;
for service in &sdt.services {
    println!("service_id = {}", service.service_id);
}
```

## Features

Default: `chrono`, `ts`, `smallvec`, `serde`.

```toml
dvb_si = { version = "0.1", default-features = false }  # tight build
dvb_si = { version = "0.1", features = ["rayon"] }       # bulk tooling
```

## Authoritative reference

ETSI EN 300 468 v1.19.1 (2025-02) — Specification for Service Information (SI)
in DVB systems. The structured table/descriptor reference transcribed from the
ETSI standards lives under [`docs/`](docs/); the canonical PDFs are vendored in
the workspace `specs/` directory.

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
