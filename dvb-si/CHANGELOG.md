# Changelog

## 1.1.0 — 2026-06-04

**Coverage milestone: every allocated `descriptor_tag` in EN 300 468 V1.19.1
Table 12 (0x40–0x7F) is implemented** — 41 new descriptor modules, each with
a symmetric `Parse`/`Serialize` pair, spec-cited layout, and round-trip tests.

### Added

**Descriptors** (EN 300 468 unless noted)
- 0x42 stuffing, 0x45 VBI_data, 0x46 VBI_teletext, 0x49 country_availability,
  0x4B NVOD_reference, 0x4C time_shifted_service, 0x4F time_shifted_event,
  0x51 mosaic (typed cell/elementary-cell loops + cell_linkage variants),
  0x53 CA_identifier, 0x57 telephone, 0x5B–0x5E multilingual
  network_name/bouquet_name/service_name/component, 0x5F
  private_data_specifier, 0x60 service_move, 0x61 short_smoothing_buffer,
  0x63 partial_transport_stream, 0x64 data_broadcast, 0x65 scrambling,
  0x66 data_broadcast_id, 0x67 transport_stream, 0x68 DSNG, 0x69 PDC,
  0x6B ancillary_data, 0x6C cell_list, 0x6D cell_frequency_link,
  0x6E announcement_support, 0x70 adaptation_field_data,
  0x72 service_availability, 0x7B DTS (Annex G), 0x7C AAC (Annex H),
  0x7E FTA_content_management
- 0x6F application_signalling, 0x71 service_identifier (TS 102 809)
- 0x74 related_content, 0x75 TVA_id (TS 102 323)
- 0x77 time_slice_fec_identifier, 0x78 ECM_repetition_rate (EN 301 192)
- 0x7D XAIT_location (TS 102 727, newly vendored)
- 0x7F extension descriptor — typed `descriptor_tag_extension` discriminant
  with 14 typed extension bodies (T2/C2/C2-bundle/S2X delivery systems,
  supplementary_audio, network_change_notify, message, target_region(_name),
  service_relocated, URI_linkage, AC-4, audio_preselection, TTML_subtitling)
  and a raw-preserving fallthrough: unknown tag_extensions round-trip
  byte-exact.

**Text** — full Annex A Table A.3 selector coverage: 0x12 KS X 1001 (EUC-KR),
0x13 GB-2312 (via GBK), 0x14 Big5, 0x1F encoding_type_id escape; two-byte
control codes (U+E080–U+E09F, Table A.2) now honored alongside the
single-byte set.

### Fixed (audit round 5, pre-release)
- extension/S2X_satellite_delivery_system (0x7F/0x17): byte-1 bit layout
  corrected to Table 140 — S2X_mode at bits [7:6] and
  scrambling_sequence_selector at bit [5] (were [4:3] and [2]).
- extension/C2_bundle_delivery_system (0x7F/0x16): bundle entries are 8 bytes
  per Table 139, not 9 — multi-entry descriptors no longer misalign.
- extension/supplementary_audio (0x7F/0x06): flags-byte bit 1 is plain
  `reserved_future_use` → now serialized as 1 per the crate convention.
- Doc-cite corrections: time_shifted_event §6.2.44, time_shifted_service
  §6.2.45, multilingual_component §6.2.23, multilingual_service_name §6.2.25,
  related_content §10.4.1.

## 1.0.1 — 2026-06-04

Docs-only: README rewritten around an explicit implementation matrix — per
table_id status, carousel message coverage, typed-descriptor list, spec
grounding. No code changes.

## 1.0.0 — 2026-06-04

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
  `Container` (0x75), `Rct` (0x76), `Cit` (0x77), `Rnt` (0x79) (TS 102 323)
- MPE family: `MpeDatagramSection` (0x3E, EN 301 192 §7 — typed IP/MAC view),
  `MpeFec` (0x78, EN 301 192 §9.9), `MpeIfec` (0x7A, TS 102 772)
- `ProtectionMessageSection` (0x7B, TS 102 809 §9) — authentication-message
  and certificate-collection variants discriminated by table_id_extension
- `DownloadableFontInfoSection` (0x7C, EN 303 560; table_id per EN 300 468
  V1.19.1 Table 2 NOTE 2 — the spec's own 0x4C was an acknowledged accident)
- **Coverage milestone: every allocated table_id in EN 300 468 V1.19.1
  Table 2 is implemented.**
- `carousel` module — DSM-CC data-carousel download protocol (ISO/IEC
  13818-6 §7.2/§7.3, DVB-profiled): typed `Dsi`/`Dii`/`DownloadDataBlock`
  messages over the `tables::dsmcc` section framing, plus
  `ModuleReassembler` for DDB→module collection. Layout provenance in
  `docs/iso_13818_6_carousel.md`; pinned against the live m6-single.ts
  capture and round-tripped byte-exact against broadcast bytes.

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

**Feature flags** — `chrono`, `ts`, `serde`.

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
- Doc-cite fixes: `teletext` type coding is Table 102 (not 99); `bouquet_name`
  §6.2.6, `network_name` §6.2.28, `parental_rating` §6.2.30, `service_list`
  §6.2.36, `stream_identifier` §6.2.41.
- `St`: accepts any `data_byte` value per §5.2.8 ("may take any value and has
  no meaning") — previously rejected non-zero stuffing, breaking real 0xFF fill.
- `Tot`: serialize emits `section_syntax_indicator = 0` per §5.2.6 (the TOT
  exception: SSI=0 yet CRC_32 present); was emitting SSI=1.
- `Sat`: PID corrected to 0x001B (EN 300 468 Table 1) — was wrongly 0x0010
  (the NIT PID); `pid::well_known::SAT` added.
- `Cat`: descriptor loop now preserved raw (`descriptors: Vec<u8>`) so non-CA
  descriptors round-trip; typed CA view via `Cat::ca_descriptors()`.
- `Cit`: dropped the desync-prone `prepend_strings_length` field (derived from
  the slice on serialize, guarded ≤ 255).
- `Bat`: no longer verifies CRC inside `parse` — crate-wide contract is that
  CRC validation belongs to `Section::validate_crc`; BAT was the lone
  inconsistent exception.
- `Nit`/`Bat`: per-entry `BufferTooShort` had `need`/`have` swapped.
- Removed the advertised-but-unimplemented `smallvec` and `rayon` feature
  flags (no code used either dependency).
- Text decoding: the default Latin table is now glyph-for-glyph faithful to
  EN 300 468 V1.19.1 Figure A.1 (transcribed in `docs/en_300_468.md`). Notable
  corrections: 0xA4 → € U+20AC (was ¤), 0xFC → þ (was œ), 0xFD → ŧ (was ı),
  0xFF → soft hyphen; quotes/arrows/fractions and the D/E/F rows no longer
  fall back to Latin-1. The full non-spacing row (macron, breve, dot, ring,
  double acute, ogonek, caron) is handled with precomposed forms and a
  base + combining-mark fallback. 0xA8 = ¤ confirmed correct against the PDF.

### Notes
- Tables and descriptors parse their outer structure with typed fields; nested
  descriptor and repeated loops are borrowed as raw `&[u8]` slices for the
  consumer to walk with the descriptor parsers.
