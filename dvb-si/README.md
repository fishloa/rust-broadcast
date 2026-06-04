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
- **Container** (0x75), **RCT** (0x76), **CIT** (0x77), **RNT** (0x79) — TS 102 323 TV-Anytime
- **MPE datagram_section** (0x3E, EN 301 192 §7 — typed IP/MAC view of the DSM-CC private section)
- **MPE-FEC** (0x78, EN 301 192 §9.9), **MPE-IFEC** (0x7A, TS 102 772)
- **Protection message** (0x7B, TS 102 809 §9 — authentication + certificate collection variants)
- **Downloadable font info / DFIS** (0x7C, EN 303 560)

With these, **every allocated table_id in EN 300 468 V1.19.1 Table 2 is
implemented** — the remaining values are reserved or user-defined.

**DSM-CC data carousel** (`carousel` module)
- Typed U-N download messages on top of the DSM-CC section framing:
  **DSI** / **DII** (table_id 0x3B) and **DDB** (0x3C), ISO/IEC 13818-6
  §7.2/§7.3 as profiled by DVB (TR 101 202, TS 102 006, TS 102 809)
- **`ModuleReassembler`** — collects DDB blocks into complete modules per the
  DII geometry (version-aware, size-capped, out-of-order tolerant)
- Layouts documented in [`docs/iso_13818_6_carousel.md`](docs/iso_13818_6_carousel.md)
  with provenance notes (ISO/IEC 13818-6 cannot be vendored) and pinned
  against a live French-TNT capture

**Descriptors** (parsed into typed structs; others pass through as raw bytes)
- 0x09 CA, 0x0A ISO-639 language, 0x40 network_name, 0x41 service_list,
  0x43 satellite_delivery_system, 0x44 cable_delivery_system, 0x48 service,
  0x4A linkage, 0x4D short_event, 0x4E extended_event, 0x50 component,
  0x54 content, 0x55 parental_rating, 0x58 local_time_offset, 0x59 subtitling,
  0x5A terrestrial_delivery_system, 0x06 data_stream_alignment, 0x73 default_authority,
  0x76 content_identifier, 0x79 S2_satellite_delivery_system, 0x6A AC-3,
  0x7A Enhanced AC-3, plus bouquet_name, logical_channel, private_data_indicator,
  registration, stream_identifier, teletext, frequency_list.

**Text** — Annex A: the default Latin table, glyph-for-glyph per Figure A.1
(ISO 6937 superset, € at 0xA4; full non-spacing diacritic row with precomposed
forms + combining-mark fallback), ISO 8859-n (via `encoding_rs`), UTF-8
(selector 0x15), UCS-2 BE (0x11); Annex A.2 control codes. Figure A.1 is
transcribed in `docs/en_300_468.md`.

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

// `section_bytes`: one complete SDT section, e.g. from `SectionReassembler`.
let sdt = Sdt::parse(section_bytes)?;
for service in &sdt.services {
    println!("service_id = {}", service.service_id);
}
```

## Features

Default: `chrono`, `ts`, `serde`.

```toml
dvb-si = { version = "0.1", default-features = false }  # tight build
```

## Authoritative reference

ETSI EN 300 468 v1.19.1 (2025-02) — Specification for Service Information (SI)
in DVB systems. The structured table/descriptor reference transcribed from the
ETSI standards lives under [`docs/`](docs/); the canonical PDFs are vendored in
the workspace `specs/` directory.

## License

Licensed under either of MIT ([LICENSE-MIT](../LICENSE-MIT)) or Apache-2.0
([LICENSE-APACHE](../LICENSE-APACHE)), at your option.
