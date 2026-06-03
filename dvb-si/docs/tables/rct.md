# RCT — Related Content Table (table_id 0x76)

**Spec:** ETSI TS 102 323 v1.4.1 §10.4 (Carriage and signalling of TV-Anytime
information). Signals links to related material for a service.
**table_id:** 0x76
**PID:** signalled — an RCT sub_table is carried in the ES whose PID is named by a
`related_content_descriptor` in that service's PMT; stream_type 0x05 (private sections).
**Parser file:** `dvb-si/src/tables/rct.rs`
**Rust struct:** `Rct`

Hand-transcribed from the canonical PDF (`specs/etsi_ts_102_323_v01.04.01_dvb_tvanytime.pdf`,
p96-97).

## Tables

### Table 109 — Related content section (§10.4.2)

| Syntax | No. of bits | Identifier |
|---|---|---|
| `related_content_section() {` |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;table_id_extension_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;section_length | 12 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;year_offset | 16 | uimsbf |
| &nbsp;&nbsp;link_count | 8 | uimsbf |
| &nbsp;&nbsp;`for (j=0; j<link_count; j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;link_info_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;link_info() | variable |  |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;reserved_future_use | 4 | bslbf |
| &nbsp;&nbsp;descriptor_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (k=0; k<descriptor_loop_length; k++) { descriptor() }` |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| `}` |  |  |

### Table 110 — Link info structure (§10.4.3)

| Syntax | No. of bits | Identifier |
|---|---|---|
| `link_info() {` |  |  |
| &nbsp;&nbsp;link_type | 4 | uimsbf |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf |
| &nbsp;&nbsp;how_related_classification_scheme_id | 6 | uimsbf |
| &nbsp;&nbsp;term_id | 12 | uimsbf |
| &nbsp;&nbsp;group_id | 4 | uimsbf |
| &nbsp;&nbsp;precedence | 4 | uimsbf |
| &nbsp;&nbsp;`if (link_type == 0x00 || link_type == 0x02) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;media_uri_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<media_uri_length; k++) { media_uri_byte` | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`if (link_type == 0x01 || link_type == 0x02) { dvb_binary_locator() }` |  |  |
| &nbsp;&nbsp;reserved_future_use | 2 | bslbf |
| &nbsp;&nbsp;number_items | 6 | uimsbf |
| &nbsp;&nbsp;`for (m=0; m<number_items; m++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;ISO_639-2_language_code | 24 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;promotional_text_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (n=0; n<promotional_text_length; n++) { promotional_text_char` | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;default_icon_flag | 1 | bslbf |
| &nbsp;&nbsp;icon_id | 3 | uimsbf |
| &nbsp;&nbsp;descriptor_loop_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (p=0; p<descriptor_loop_length; p++) { descriptor() }` |  |  |
| `}` |  |  |

## Field semantics (key)

- **table_id_extension_flag** — 0: table_id_extension carries service_id (sections
  relate to one service); 1: all sections relate to a single service and service_id is ignored.
- **year_offset** — the year (binary, e.g. 0x07D3 = 2003) relative to which date
  values in the section are calculated.
- **link_type** — format of the link info: URI (e.g. a CRID), a DVB binary locator,
  both, or a descriptor.

---
_Hand-transcribed from ETSI TS 102 323 v1.4.1 §10.4 (PDF pp. 96-97)._
