# CIT — Content Identifier Table (table_id 0x77)

**Spec:** ETSI TS 102 323 v1.4.1 §12.2 (Carriage and signalling of TV-Anytime
information). Maps content reference identifiers (CRIDs) to events.
**table_id:** 0x77
**PID:** 0x0012 (CIT sections are carried on PID 0x0012, shared with the EIT).
**Parser file:** `dvb-si/src/tables/cit.rs`
**Rust struct:** `Cit`

Hand-transcribed from the canonical PDF (`specs/etsi_ts_102_323_v01.04.01_dvb_tvanytime.pdf`,
p105-106).

## Tables

### Table 119 — Content identifier section (§12.2)

| Syntax | No. of bits | Identifier |
|---|---|---|
| `content_identifier_section() {` |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;private_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;section_length | 12 | uimsbf |
| &nbsp;&nbsp;service_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;transport_stream_id | 16 | uimsbf |
| &nbsp;&nbsp;original_network_id | 16 | uimsbf |
| &nbsp;&nbsp;prepend_strings_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<prepend_strings_length; i++) { prepend_strings_byte` | 8 | uimsbf |
| &nbsp;&nbsp;`for (j=0; j<N; j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;crid_ref | 16 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;prepend_string_index | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;unique_string_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<unique_string_length; k++) { unique_string_byte` | 8 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| `}` |  |  |

## Field semantics (key)

- **service_id** — the `container_id` of the container this section belongs to.
- **prepend_strings** — a concatenation of prepend strings partitioned by a 0x00
  byte; referenced by index (first = 0).
- **crid_ref** — reference value for a CRID, referenced from the content_identifier_descriptor of the EIT.
- **prepend_string_index** — index of the prepend_string to prefix; 0xFF = none
  (unique_string holds the full CRID). The common `CRID://` prefix may be implied.

---
_Hand-transcribed from ETSI TS 102 323 v1.4.1 §12.2 (PDF pp. 105-106)._
