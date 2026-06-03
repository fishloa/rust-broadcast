# RNT — Resolution provider Notification Table (table_id 0x79)

**Spec:** ETSI TS 102 323 v1.4.1 §5.2.2 (RAR over DVB — Carriage and signalling
of TV-Anytime information). Carries the locations of CRI (Content Referencing
Information) and metadata for CRID authorities.
**table_id:** 0x79
**PID:** 0x0016 (RNT sections are carried on PID 0x0016).
**Parser file:** `dvb-si/src/tables/rnt.rs`
**Rust struct:** `Rnt`

Hand-transcribed from the canonical PDF (`specs/etsi_ts_102_323_v01.04.01_dvb_tvanytime.pdf`,
p17).

## Tables

### Table 1 — Resolution Provider Notification Section (§5.2.2)

| Syntax | No. of bits | Identifier |
|---|---|---|
| `resolution_authority_notification_section() {` |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;section_length | 12 | uimsbf |
| &nbsp;&nbsp;context_id | 16 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;version_number | 5 | uimsbf |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf |
| &nbsp;&nbsp;section_number | 8 | uimsbf |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |
| &nbsp;&nbsp;context_id_type | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;common_descriptors_length | 12 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N1; i++) { descriptor() }` |  |  |
| &nbsp;&nbsp;`for (i=0; i<N2; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;resolution_provider_info_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;resolution_provider_name_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<…; j++) { resolution_provider_name_byte` | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;resolution_provider_descriptors_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<N3; j++) { descriptor() }` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<N4; j++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CRID_authority_name_length | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<…; k++) { CRID_authority_name_byte` | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CRID_authority_policy | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;CRID_authority_descriptors_length | 12 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<N5; k++) { CRID_authority_descriptor() }` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |
| `}` |  |  |

## Field semantics (key)

- **context_id / context_id_type** — identify the context this sub_table applies
  to. Table 2: 0x00 = bouquet_id, 0x01 = original_network_id, 0x02 = network_id,
  0x03–0x7F DVB reserved, 0x80–0xFF user defined.
- **resolution_provider_name** — a registered internet domain name (DNS), case-insensitive.
- **CRID_authority_policy** — Table 3: '00' permanent (never re-used), '01' transient,
  '10' either, '11' reserved.

---
_Hand-transcribed from ETSI TS 102 323 v1.4.1 §5.2.2 (PDF p. 17)._
