# Compact SCTE 35 Encoding Format

_Source: ETSI TS 103 752-1 V1.2.1 §8.3.3, Tables 5–10 (PDF pp.26–28), render-verified_

**NEW binary syntax.** Watermark carriers can have very limited data capacity, so
the spec defines an **optional compact alternative** to the full SCTE 35 message
for conveying *certain* SCTE 35 messages via watermark messages (§8.3.3). It
encodes only the fields that vary; all other base SCTE 35 /
`segmentation_descriptor()` / `DVB_DAS_descriptor()` fields are **presumed** to
take the values fixed by the §5.3 profile (the implied-value Tables 7, 9, 10).

The compact format is used in watermark carriage only. The non-compact
("Standard Encoding Format", §8.3.2) reuses the full SCTE 35 message profile of
§5.3 unchanged.

> ⚠ **Bit-width caveat (whole document).** The "No. of Bits" column of Tables 6
> and 8 in the PDF render is vertically mis-registered against the syntax rows in
> several places (the values column drifts relative to the field names). Each
> table below transcribes the rendered values **and** gives the most-likely
> intended widths cross-checked against §5.3.5.11 and the `DVB_DAS_descriptor()`
> (Table 1, see [`das-descriptor.md`](das-descriptor.md)). **Verify Tables 6 and 8
> bit widths against a clean copy of the PDF before implementing.**

## Table 5 — `compact_SCTE_35()`

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `compact_SCTE_35() {` | | |
| &nbsp;&nbsp;message_type | 8 | uimsbf |
| &nbsp;&nbsp;`if (message_type == 0x00) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`compact_time_signal()` | | |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`else if (message_type == 0x01) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`compact_splice_insert()` | | |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

- **message_type** (8) — `0x00` ⇒ `compact_time_signal()` (Table 6); `0x01` ⇒
  `compact_splice_insert()` (Table 8). All other values reserved for future use.

## Table 6 — `compact_time_signal()`

Rendered (source order of the "No. of Bits" column: `1, 6, 8, 33, 32, 40, 8, 8,
N*8, 8, 8, 8`):

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `compact_time_signal() {` | | |
| &nbsp;&nbsp;encrypted_packet | 1 | bslbf |
| &nbsp;&nbsp;encryption_algorithm | 6 | uimsbf |
| &nbsp;&nbsp;cw_index | 8 | uimsbf |
| &nbsp;&nbsp;pts_time | 33 | uimsbf |
| &nbsp;&nbsp;segmentation_event_id | 32 | uimsbf |
| &nbsp;&nbsp;segmentation_duration | 40 | uimsbf |
| &nbsp;&nbsp;segmentation_type_id | 8 | uimsbf |
| &nbsp;&nbsp;segmentation_upid_length (N) | 8 | uimsbf |
| &nbsp;&nbsp;segmentation_upid | N*8 | uimsbf |
| &nbsp;&nbsp;segments_num | 8 | uimsbf |
| &nbsp;&nbsp;segments_expected | 8 | uimsbf |
| &nbsp;&nbsp;`if (encrypted_packet) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;E_CRC_32 | 32 ⚠ | rpchof |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

⚠ The render's value column has 12 entries (`1,6,8,33,32,40,8,8,N*8,8,8,8`) for
the 12 value-bearing rows down to `segments_expected`, then the trailing `E_CRC_32
(rpchof)` row shows a value of `8` in the render. **`E_CRC_32` is a 32-bit CRC
(`rpchof`)** — the rendered `8` is a mis-registration; transcribed here as **32**.

Semantics (§8.3.3): "Unless otherwise noted, all fields of the
`compact_time_signal()` shall have the same semantic as the corresponding field in
the `time_signal()` as specified in the SCTE 35 specification subject to the
constraints given in §5.3."

### Table 7 — Implied `segmentation_descriptor` fields in `compact_time_signal()`

Fields NOT explicitly conveyed in the compact encoding are presumed to take these
values (§8.3.3):

| Field | Value |
|-------|-------|
| segmentation_event_cancel_indicator | 0 |
| segmentation_duration_flag | 1 |
| time_specified_flag | 1 |
| segmentation_upid_type | 0x0F |
| program_segmentation_flag | 1 |
| segment_delivery_not_restricted_flag | 1 |

## Table 8 — `compact_splice_insert()`

Rendered (source order of the "No. of Bits" column: `1, 6, 8, 33, 32, 33, 33, 16,
8, 8, 1, 6, 8, 8, 8, 4, 4, (N-3)*8, 32`):

| Syntax | No. of bits (render) | No. of bits (likely) | Mnemonic |
|--------|----------------------|----------------------|----------|
| `compact_splice_insert() {` | | | |
| &nbsp;&nbsp;encrypted_packet | 1 | 1 | bslbf |
| &nbsp;&nbsp;encryption_algorithm | 6 | 6 | uimsbf |
| &nbsp;&nbsp;cw_index | 8 | 8 | uimsbf |
| &nbsp;&nbsp;pts_time | 33 | 33 | uimsbf |
| &nbsp;&nbsp;splice_event_id | 32 | 32 | uimsbf |
| &nbsp;&nbsp;duration | 33 ⚠ | 33 | uimsbf |
| &nbsp;&nbsp;unique_program_id | 33 ⚠ | **16** ⚠ | uimsbf |
| &nbsp;&nbsp;avail_num | 16 ⚠ | **8** ⚠ | uimsbf |
| &nbsp;&nbsp;avails_expected | 8 | 8 | uimsbf |
| &nbsp;&nbsp;DAS_descriptor_flag | 8 ⚠ | **1** ⚠ | uimsbf/bslbf |
| &nbsp;&nbsp;reserved | 1 ⚠ | **7** ⚠ | bslbf |
| &nbsp;&nbsp;`if (DAS_descriptor_flag) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptor_length (N) | 6 ⚠ | 6 (or 8) | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;break_num | 8 | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;breaks_expected | 8 | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;equivalent_segmentation_type | 8 ⚠ | **4** ⚠ | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 4 | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;upid() | 4 ⚠ | **(N-3)*8** ⚠ | uimsbf |
| &nbsp;&nbsp;`}` | | | |
| &nbsp;&nbsp;`if (encrypted_packet) {` | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;E_CRC_32 | (N-3)*8 ⚠ | **32** ⚠ | rpchof |
| &nbsp;&nbsp;`}` | | | |
| `}` | | | |

⚠ **Table 8 is the most mis-registered table in the render** — the "No. of Bits"
column has drifted down by roughly one row across the lower half. The
"likely" column is reconstructed by cross-referencing:
- §5.3.5.11: `unique_program_id` in `splice_insert()` is a **16-bit** field
  (the rendered `33` is implausible and shifted from `duration`'s 33).
- SCTE 35 base: `avail_num` / `avails_expected` are **8-bit** each.
- `DAS_descriptor_flag` is a **flag (1 bit)** followed by a reserved span; the
  rendered `8`/`1` pair is shifted.
- The DAS-descriptor body inside the `if` block mirrors Table 1 of
  [`das-descriptor.md`](das-descriptor.md): `break_num` (8), `breaks_expected`
  (8), `equivalent_segmentation_type` (**4**), `reserved` (4), then `upid()` as
  the variable-length trailer.
- `E_CRC_32` is a **32-bit** CRC (`rpchof`); the rendered `(N-3)*8` value belongs
  to the `upid()` row.

**Do not implement `compact_splice_insert()` bit widths from this transcription
without re-verifying against a clean PDF or the published erratum — the render
alignment is unreliable for this table.**

Semantics (§8.3.3):
- **pts_time** (33) — media time of the splice event on the timeline of the
  watermark technology in which the message is conveyed.
- **upid** — ASCII-encoded variable-length field conveying either: (a) an 8-byte
  Airing ID (associated domain name determined in a watermark-technology-specific
  manner; Airing ID format per §5.3); or (b) a reverse domain name with `:`
  -separated fields followed by `/` and a UUID — for case (b) no prefix is
  included (implied prefix is `"urn:"`); UUID format per §5.3.
- All other fields share the semantics of the corresponding `splice_insert()` /
  `DVB_DAS_descriptor()` fields, subject to the §5.3 constraints.

### Table 9 — Implied `segmentation_descriptor` fields in `compact_splice_insert()`

| Field | Value |
|-------|-------|
| splice_event_cancel_indicator | 0 |
| out_of_network_indicator | 1 |
| duration_flag | 1 |
| splice_immediate_flag | 0 |
| time_specified_flag | 1 |
| segmentation_upid_type | 0x0F |
| auto_return | 1 |
| program_splice_flag | 1 |

### Table 10 — Implied `DVB_DAS_descriptor` fields in `compact_splice_insert()`

| Field | Value |
|-------|-------|
| splice_descriptor_tag | 0xF0 |
| identifier | 0x4456425F (ASCII `"DVB_"`) |
