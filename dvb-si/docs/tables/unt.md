# UNT — Update Notification Table (table_id 0x4B)

**Spec:** ETSI TS 102 006 v1.4.1 §9.4 (System Software Update)
**table_id:** 0x4B
**PID:** signalled — the UNT is referenced by a `data_broadcast_id_descriptor`
(data_broadcast_id = 0x000A) in the PMT ES_info loop; no fixed PID.
**Parser file:** `dvb-si/src/tables/unt.rs`
**Rust struct:** `Unt`

Hand-transcribed from the canonical PDF (`specs/etsi_ts_102_006_v01.07.01_dvb_ssu.pdf`,
p22-26) — the pdfplumber extraction misaligns the bit-width column on the
brace-bearing syntax rows, so widths below are read from the page image.

## Tables

### Table 11 — Syntax of the Update Notification Section (§9.4.1)

| Syntax | No. of bits | Identifier | Default / remark |
|---|---|---|---|
| `Update_Notification_Table() {` |  |  |  |
| &nbsp;&nbsp;table_id | 8 | uimsbf | 0x4B |
| &nbsp;&nbsp;section_syntax_indicator | 1 | bslbf | 1 |
| &nbsp;&nbsp;reserved_for_future_use | 1 | bslbf | 1 |
| &nbsp;&nbsp;reserved | 2 | bslbf | 11 |
| &nbsp;&nbsp;section_length | 12 | uimsbf | max 0xFFD |
| &nbsp;&nbsp;action_type | 8 | uimsbf | 0x01 |
| &nbsp;&nbsp;OUI_hash | 8 | uimsbf |  |
| &nbsp;&nbsp;reserved | 2 | bslbf | 11 |
| &nbsp;&nbsp;version_number | 5 | uimsbf |  |
| &nbsp;&nbsp;current_next_indicator | 1 | bslbf | 1 |
| &nbsp;&nbsp;section_number | 8 | uimsbf |  |
| &nbsp;&nbsp;last_section_number | 8 | uimsbf |  |
| &nbsp;&nbsp;OUI | 24 | uimsbf |  |
| &nbsp;&nbsp;processing_order | 8 | uimsbf |  |
| &nbsp;&nbsp;common_descriptor_loop() | variable |  | §9.4.2.1 |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;compatibilityDescriptor() | variable |  | §9.4.2.2 |
| &nbsp;&nbsp;&nbsp;&nbsp;platform_loop_length | 16 | uimsbf |  |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<N; i++) {` |  |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;target_descriptor_loop() | variable |  | §9.4.2.3 |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;operational_descriptor_loop() | variable |  | §9.4.2.4 |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` |  |  |  |
| &nbsp;&nbsp;`}` |  |  |  |
| &nbsp;&nbsp;CRC_32 | 32 | rpchof |  |
| `}` |  |  |  |

### Table 14 / 17 / 18 — descriptor loops (§9.4.2.1, §9.4.2.3, §9.4.2.4)

`common_descriptor_loop()`, `target_descriptor_loop()` and
`operational_descriptor_loop()` share the same shape:

| Syntax | No. of bits | Identifier |
|---|---|---|
| reserved | 4 | bslbf |
| &lt;loop&gt;_descriptor_loop_length | 12 | uimsbf |
| `for (i=0; i<N; i++) { descriptor() }` | variable |  |

### Table 15 — compatibilityDescriptor() structure (§9.4.2.2)

A length-prefixed block (NOT a tag/length SI descriptor — ISO/IEC 13818-6
groupInfo form):

| Syntax | No. of bytes |
|---|---|
| compatibilityDescriptorLength | 2 |
| descriptorCount | 2 |
| `for (i=0; i<N; i++) {` |  |
| &nbsp;&nbsp;descriptorType | 1 |
| &nbsp;&nbsp;descriptorLength | 1 |
| &nbsp;&nbsp;specifierType | 1 |
| &nbsp;&nbsp;specifierData | 3 |
| &nbsp;&nbsp;model | 2 |
| &nbsp;&nbsp;version | 2 |
| &nbsp;&nbsp;subDescriptorCount | 1 |
| &nbsp;&nbsp;`for (i=0; i<N; i++) { subDescriptor() }` |  |
| `}` |  |

## Field semantics (key)

- **action_type** — Table 12: 0x00 reserved, 0x01 System Software Update,
  0x02–0x7F reserved, 0x80–0xFF user defined.
- **OUI_hash** — `OUI[23..16] ^ OUI[15..8] ^ OUI[7..0]` (XOR of the three OUI bytes).
- **OUI** — IEEE Organizationally Unique Identifier forming the sub-table index.
  The DVB-reserved generic OUI `0x00015A` indicates selection only by analysing the UNT.
- **processing_order** — Table 13: 0x00 first action, 0x01–0xFE subsequent
  (ascending), 0xFF no ordering implied.
- **platform_loop_length** — combined length of the following
  `target_descriptor_loop()` + `operational_descriptor_loop()`.

---
_Hand-transcribed from ETSI TS 102 006 v1.4.1 §9.4 (PDF pp. 22-26)._
