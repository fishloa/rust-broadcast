# INT — IP/MAC Notification Table (table_id 0x4C)

**Spec:** ETSI EN 301 192 v1.7.1 §8.4 (DVB specification for data broadcasting)
**table_id:** 0x4C
**PID:** signalled — the INT is referenced by a `data_broadcast_id_descriptor`
(data_broadcast_id = 0x000B) in the PMT ES_info loop; no fixed PID.
**Parser file:** `dvb-si/src/tables/int.rs`
**Rust struct:** `Int`

Hand-transcribed from the canonical PDF (`specs/etsi_en_301_192_v01.07.01_dvb_databcast.pdf`,
p25-28).

## Tables

### Table 13 — Syntax of the IP/MAC_notification_section (§8.4.4.0)

| Syntax | No. of bits | Identifier | Remark |
|---|---|---|---|
| `IP/MAC_notification_section () {` |  |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf | 0x4C |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf | 1 |
| &nbsp;&nbsp;reserved_for_future_use | 1 | bslbf | 1 |
| &nbsp;&nbsp;reserved | 2 | bslbf | 11 |
| &nbsp;&nbsp;section_length | 12 | uimsbf | max 4093 |
| &nbsp;&nbsp;action_type | 8 | uimsbf | Table 14 |
| &nbsp;&nbsp;platform_id_hash | 8 | uimsbf |  |
| &nbsp;&nbsp;reserved | 2 | bslbf | 11 |
| &nbsp;&nbsp;version_number | 5 | uimsbf |  |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf | 1 |
| &nbsp;&nbsp;section_number | 8 | uimsbf |  |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |  |
| &nbsp;&nbsp;platform_id | 24 | uimsbf |  |
| &nbsp;&nbsp;processing_order | 8 | uimsbf | 0x00 |
| &nbsp;&nbsp;platform_descriptor_loop() | variable |  |  |
| &nbsp;&nbsp;`for (i=0, i<N1, i++) {` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;target_descriptor_loop() | variable |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;operational_descriptor_loop() | variable |  |  |
| &nbsp;&nbsp;`}` |  |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |  |
| `}` |  |  |  |

Each descriptor loop is `reserved (4) + <loop>_descriptor_loop_length (12) +
descriptor()*` (same shape as the UNT loops).

### Table 12 — IP/MAC_notification_info structure (§8.3.1, data_broadcast_id selector 0x000B)

| Syntax | No. of bits | Identifier |
|---|---|---|
| `IP/MAC_notification_info () {` |  |  |
| &nbsp;&nbsp;platform_id_data_length | 8 | uimsbf |
| &nbsp;&nbsp;`for (i=0; i<N; i++){` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;platform_id | 24 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;action_type | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;INT_versioning_flag | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;INT_version | 5 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`for (i=0; i<N; i++){ private_data_byte` | 8 | uimsbf |
| `}` |  |  |

## Field semantics (key)

- **platform_id** — 24-bit label uniquely identifying the IP/MAC platform (TS 101 162).
- **platform_id_hash** — XOR hash of the platform_id, forming part of the
  table_id_extension for fast section filtering. Not unique → use the full platform_id.
- **action_type** — 0x01 = IP/MAC stream announcement/location.
- **target_descriptor_loop** — if non-empty, only explicitly-targeted receivers act;
  if empty, all receivers under the platform_id are concerned.

---
_Hand-transcribed from ETSI EN 301 192 v1.7.1 §8.3-§8.4 (PDF pp. 25-28)._
