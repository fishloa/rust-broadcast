# Changelog

## 0.1.0 — unreleased

First substantive release covering the MPEG-2 PSI and DVB SI table set, the
common descriptors, and the DVB-allocated companion tables.

### Added

**Framing**
- `Section<'a>` — long/short-form PSI/SI section header with CRC-32 validation
- `TsPacket<'a>` + `SectionReassembler` under feature `ts`

**Tables** (each with `Parse` + `Serialize` round-trip tests)
- MPEG-2 PSI: `Pat` (0x00), `Cat` (0x01), `Pmt` (0x02), `Tsdt` (0x03)
- DVB SI: `Nit` (0x40/0x41), `Sdt` (0x42/0x46), `Bat` (0x4A), `Eit` (0x4E–0x6F),
  `Tdt` (0x70), `Rst` (0x71), `St` (0x72), `Tot` (0x73), `Dit` (0x7E), `Sit` (0x7F)
- `Sat` — Satellite Access Table family (0x4D, §5.2.11)
- `Ait` (0x74, TS 102 809), `DsmccSection` (0x3A–0x3F)
- Companion tables: `Unt` (0x4B, TS 102 006), `Int` (0x4C, EN 301 192),
  `Rct` (0x76), `Cit` (0x77), `Rnt` (0x79) (TS 102 323)

**Descriptors** — typed parsers for the common DVB + MPEG-2 descriptors
(network_name, service, service_list, linkage, short/extended_event, component,
content, parental_rating, CA, satellite/cable/terrestrial/S2 delivery system,
local_time_offset, subtitling, teletext, AC-3 / Enhanced AC-3, logical_channel,
default_authority, content_identifier, registration, stream_identifier,
data_stream_alignment, frequency_list, bouquet_name, private_data_indicator,
iso_639_language). Descriptors not yet typed pass through as raw bytes.

**Text** — Annex A subset: ISO 6937 (with diacritic combining), ISO 8859-n,
UTF-8 (selector 0x15), UCS-2 BE (0x11); Annex A.2 control codes.

**CRC** — Annex C MPEG-2 CRC-32 (from `dvb_common`).

**Typed constants** — `TableId`, `DescriptorTag`, `pid::well_known`.

**Feature flags** — `chrono`, `ts`, `smallvec`, `serde`, `rayon`.

### Fixed
- `Tsdt` (0x03): removed a phantom `descriptor_loop_length` field — per
  ISO/IEC 13818-1 §2.4.4.12 descriptors run directly from byte 8 to the CRC,
  bounded by `section_length`.
- `Nit` / `Sdt`: added the `section_number` / `last_section_number` fields
  (previously parsed-and-discarded, serialized as 0), so multi-section
  sub-tables round-trip faithfully.
- `St` (0x72): byte-1 reserved nibble now `0x70` (reserved_future_use = 1),
  matching the other short-form sections (DIT/RST/TDT/TOT).
- `satellite_delivery_system` (0x43): corrected table cites — Polarization
  Table 38, Roll-off Table 39, Modulation system Table 40, Modulation type
  Table 41; `symbol_rate_bcd` masked to 28 bits on serialize.
- `s2_satellite_delivery_system` (0x79): `reserved_zero_future_use` (bit 5) now
  serialized as 0 per §6.2.13.3 Table 42 (was incorrectly 1).
- `content_identifier` (0x76): dropped the unreachable `CridLocation::Reserved`
  variant (reserved locations have no defined length and are rejected on parse).
- Doc-cite fixes: `teletext` type coding is Table 102 (not 99).

### Notes
- Tables and descriptors parse their outer structure with typed fields; nested
  descriptor and repeated loops are borrowed as raw `&[u8]` slices for the
  consumer to walk with the descriptor parsers.
