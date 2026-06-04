# dvb-si

ETSI EN 300 468 DVB Service Information parser **and builder**, plus the
MPEG-2 PSI tables it builds on, the DVB-allocated companion tables, and the
DSM-CC data carousel.

**Complete table coverage: every allocated `table_id` in EN 300 468 V1.19.1
Table 2 is implemented** — 29 tables, each with a symmetric
`Parse` / `Serialize` pair and round-trip tests. Layouts are derived from the
ETSI specs (vendored in the repo and transcribed into reviewable markdown) and
validated against live broadcast captures.

## Table coverage

Every table: typed header fields, symmetric parse/serialize, round-trip
tested. Per the crate's zero-copy convention, descriptor loops and repeated
sub-structures are borrowed `&[u8]` slices the caller walks with the
descriptor parsers — noted below only where a table goes further or stays
deliberately raw.

| table_id | Table | Spec | Status |
|---|---|---|---|
| 0x00 | PAT — Program Association | ISO/IEC 13818-1 | ✅ full |
| 0x01 | CAT — Conditional Access | ISO/IEC 13818-1 | ✅ full + typed `ca_descriptors()` view |
| 0x02 | PMT — Program Map | ISO/IEC 13818-1 | ✅ full (typed ES loop) |
| 0x03 | TSDT — TS Description | ISO/IEC 13818-1 | ✅ full |
| 0x3A–0x3F | DSM-CC sections | ISO/IEC 13818-6 / EN 301 192 | ✅ framing; 0x3B/0x3C payloads typed via [`carousel`](#dsm-cc-data-carousel); 0x3E typed as MPE |
| 0x3E | MPE datagram_section (typed IP/MAC view) | EN 301 192 §7 | ✅ full (MAC reassembly, LLC/SNAP flag, SSI-aware trailer) |
| 0x40/0x41 | NIT actual/other | EN 300 468 §5.2.1 | ✅ full (typed TS loop) |
| 0x42/0x46 | SDT actual/other | EN 300 468 §5.2.3 | ✅ full (typed service loop) |
| 0x4A | BAT — Bouquet Association | EN 300 468 §5.2.2 | ✅ full (typed TS loop) |
| 0x4B | UNT — Update Notification (SSU) | TS 102 006 | ✅ full |
| 0x4C | INT — IP/MAC Notification | EN 301 192 | ✅ full |
| 0x4D | SAT — Satellite Access family | EN 300 468 §5.2.11 | ✅ header + `SatTableId` discriminant typed; variant bodies raw (bit-packed orbital data, layout in docs) |
| 0x4E–0x6F | EIT p/f + schedule, actual/other | EN 300 468 §5.2.4 | ✅ full (typed event loop; `chrono`-gated MJD+BCD `start_time()`) |
| 0x70 | TDT — Time and Date | EN 300 468 §5.2.5 | ✅ full |
| 0x71 | RST — Running Status | EN 300 468 §5.2.7 | ✅ full (typed event loop) |
| 0x72 | ST — Stuffing | EN 300 468 §5.2.8 | ✅ full |
| 0x73 | TOT — Time Offset | EN 300 468 §5.2.6 | ✅ full (incl. the SSI=0-with-CRC framing exception) |
| 0x74 | AIT — Application Information | TS 102 809 | ✅ full (typed application loop), validated vs live HbbTV capture |
| 0x75 | Container | TS 102 323 | ✅ full |
| 0x76 | RCT — Related Content | TS 102 323 | ✅ full |
| 0x77 | CIT — Content Identifier | TS 102 323 | ✅ full |
| 0x78 | MPE-FEC | EN 301 192 §9.9 | ✅ full (typed real_time_parameters) |
| 0x79 | RNT — Resolution Notification | TS 102 323 | ✅ full |
| 0x7A | MPE-IFEC | TS 102 772 | ✅ full (typed real_time_parameters) |
| 0x7B | Protection message | TS 102 809 §9 | ✅ full — authentication-message + certificate-collection variants by table_id_extension |
| 0x7C | DFIS — Downloadable Font Info | EN 303 560 | ✅ full (typed font_info loop; table_id per EN 300 468 Table 2 NOTE 2) |
| 0x7E | DIT — Discontinuity Information | EN 300 468 | ✅ full |
| 0x7F | SIT — Selection Information | EN 300 468 | ✅ full |

Remaining table_id values are *reserved* or *user-defined* in EN 300 468
V1.19.1 Table 2 — there is nothing standardized left to implement.

## DSM-CC data carousel

The `carousel` module types the download-protocol payloads carried inside
DSM-CC sections (ISO/IEC 13818-6 §7.2/§7.3 as profiled by DVB — TR 101 202,
TS 102 006 SSU, TS 102 809 object carousels):

| Message | messageId | Status |
|---|---|---|
| DSI — DownloadServerInitiate | 0x1006 | ✅ full (privateData raw: SSU GroupInfoIndication / OC ServiceGatewayInfo) |
| DII — DownloadInfoIndication | 0x1002 | ✅ full (typed module loop) |
| DDB — DownloadDataBlock | 0x1003 | ✅ full |
| `ModuleReassembler` | — | ✅ DDB → complete modules per DII geometry: version-aware, out-of-order tolerant, per-module + aggregate memory caps |

Validated **byte-exact** against a live French-TNT (M6 HbbTV) capture in the
test suite. BIOP object-carousel payloads above this layer are out of scope.

## Descriptors

29 descriptors parse into typed structs; any other tag passes through as raw
bytes (tag + payload preserved):

| Tags | Typed descriptors |
|---|---|
| MPEG-2 | 0x05 registration · 0x06 data_stream_alignment · 0x09 CA · 0x0A ISO-639 language · 0x0F private_data_indicator |
| DVB | 0x40 network_name · 0x41 service_list · 0x43 satellite_delivery · 0x44 cable_delivery · 0x47 bouquet_name · 0x48 service · 0x4A linkage · 0x4D short_event · 0x4E extended_event · 0x50 component · 0x52 stream_identifier · 0x54 content · 0x55 parental_rating · 0x56 teletext · 0x58 local_time_offset · 0x59 subtitling · 0x5A terrestrial_delivery · 0x62 frequency_list · 0x6A AC-3 · 0x73 default_authority · 0x76 content_identifier · 0x79 S2_satellite_delivery · 0x7A Enhanced AC-3 |
| Private | 0x83 logical_channel (EACEM/NorDig) |

## Text decoding

EN 300 468 Annex A: the default Latin table **glyph-for-glyph per Figure A.1**
(ISO 6937 superset — € at 0xA4, full non-spacing diacritic row with
precomposed forms + combining-mark fallback, every position pinned by tests),
ISO 8859-n via `encoding_rs`, UTF-8 (selector 0x15), UCS-2 BE (0x11), Annex
A.2 control codes.

## Spec grounding

Every layout is cited. The repo vendors the ETSI PDFs and transcribes their
syntax tables into reviewable markdown
([`docs/`](https://github.com/fishloa/rust-dvb/tree/main/dvb-si/docs)):
EN 300 468 V1.19.1 (2025-02), TS 102 323, TS 102 006, EN 301 192, TS 102 809,
TS 102 772, EN 303 560, TR 101 202 — plus a provenance-documented hand
transcription for ISO/IEC 13818-6 (not freely redistributable). The crate has
been through four adversarial spec-audit rounds; fixture tests run against
real transponder captures.

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

## Principles

- **Spec fidelity.** Every field in a section's syntax appears in the parsed struct.
- **Parse and construct.** Every parser has a symmetric serializer; round-trip is tested.
- **Zero-copy where possible.** Parsed types borrow from the input via `<'a>` lifetimes.
- **No magic numbers.** Every hex literal outside `#[cfg(test)]` is a named constant or enum.

## Features

Default: `chrono` (MJD+BCD → `DateTime<Utc>`), `ts` (TS packet +
`SectionReassembler`), `serde`.

```toml
dvb-si = { version = "1.0", default-features = false }  # tight build
```

## Family

[`dvb-common`](https://crates.io/crates/dvb-common) (traits + CRC-32) ·
[`dvb-t2mi`](https://crates.io/crates/dvb-t2mi) (T2-MI, all 12 packet types) ·
[`dvb-bbframe`](https://crates.io/crates/dvb-bbframe) (S2/S2X/T2 BBFRAME).
For GSE see the existing [`dvb-gse`](https://crates.io/crates/dvb-gse) crate.

## License

Licensed under either of MIT or Apache-2.0, at your option.
