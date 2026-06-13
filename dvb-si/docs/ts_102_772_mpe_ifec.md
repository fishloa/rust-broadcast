# ETSI TS 102 772 v1.1.1 — MPE inter-burst FEC (MPE-IFEC)

> **✓ Accuracy-verified against the PDF — 2026-06-13.** Split-cell mangling removed and cosmetic column-padding trimmed; every table cross-checked against BlazeDocs OCR and reconciled against the canonical PDF pages. Table 10's reserved range is reproduced verbatim from the spec (`01 to 11`, see the spec note there) rather than silently "corrected". Source: BlazeDocs OCR + direct PDF page verification.

Reference transcribed from the canonical PDF (`specs/etsi_ts_102_772_v01.01.01_dvb_mpe_ifec.pdf`) by the
geometry-based extractor in `tools/dvb-si-audit/` — field rows aligned to
their bit-widths by page geometry, reproduced verbatim. The PDF in `specs/`
is the authoritative source.

## Contents

- [Table 1 — MPE-IFEC generic parameters list](#table-1-mpe-ifec-generic-parameters-list)
- [Table 2 — MPE-IFEC section](#table-2-mpe-ifec-section)
- [Table 3 — Time Slicing and MPE-IFEC real time parameters](#table-3-time-slicing-and-mpe-ifec-real-time-parameters)
- [Table 4 — Time Slice and FEC identifier descriptor](#table-4-time-slice-and-fec-identifier-descriptor)
- [Table 5 — Syntax and semantics for mpe_fec](#table-5-syntax-and-semantics-for-mpe_fec)
- [Table 6 — Syntax and semantics for frame_size](#table-6-syntax-and-semantics-for-frame_size)
- [Table 7 — Syntax and semantics for max_average_rate](#table-7-syntax-and-semantics-for-max_average_rate)
- [Table 8 — Syntax and semantics for id_selector_length](#table-8-syntax-and-semantics-for-id_selector_length)
- [Table 9 — Semantics for time_slice_fec_id = 0x1](#table-9-semantics-for-time_slice_fec_id-0x1)
- [Table 10 — Syntax and semantics for T_code](#table-10-syntax-and-semantics-for-t_code)
- [Table 11 — Syntax and semantics for G_code parameter](#table-11-syntax-and-semantics-for-g_code-parameter)
- [Table 12 — Recommended parameter settings for frame_size=0x03](#table-12-recommended-parameter-settings-for-frame_size0x03)

## Table 1 — MPE-IFEC generic parameters list
_§4.4, PDF pp. 14-15_

| Parameter | Unit | Category | Description | Signalling | Scoping |
|---|---|---|---|---|---|
| EP | Datagram burst | Taxonomy | IFEC Encoding Period | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| D | Datagram burst | Taxonomy | Datagram burst sending delay | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| T | rows | Table sizing | Number of ADST, ADT, iFDT rows; T=MPE-FEC Frame rows /G | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| C | columns | Table sizing | Number of ADST columns | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| R | sections | Table sizing | Maximum number of MPE IFEC sections per Time-Slice Burst | Direct via Time_slice_fec_identifier | Time_slice_fec_identifier |
| K | columns | Table sizing | Number of ADT columns = EP*C | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| N | columns | Table sizing | Number of iFDT columns = EP*R*G | Indirect via Time_slice_fec_identifier | Time_slice_fec_identifier |
| G | columns | Table sizing | Maximum number of iFDT columns per IFEC section | Direct | Time_slice_fec_identifier |
| M | ADT | Protocol sizing | Number of concurrent encoding matrices M | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| kmax | N/A | Protocol sizing | Modulo operator for IFEC burst counter | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| lmax | N/A | Protocol sizing | Maximum backward pointing for datagram burst size used in PREV_BURST_SIZE parameter in clause 3.5 | Indirect (formula dependent on T_code and given in the parameter definition of clause 6) | Time_slice_fec_identifier |
| k | datagram burst | Index | continuous burst counter internal to sender | N/A | Loop |
| k' | IFEC burst | field | Burst number | N/A | IFEC section |

## Table 2 — MPE-IFEC section
_§5.2, PDF pp. 17-17_

| Syntax | Number of bits | Identifier |
|---|---|---|
| MPE-IFEC_section () { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | Bslbf |
| Private_indicator | 1 | Bslbf |
| Reserved | 2 | Bslbf |
| section_length | 12 | uimsbf |
| Burst_number | 8 | uimsbf |
| IFEC_burst_size | 8 | uimsbf |
| Reserved | 2 | Bslbf |
| Version | 5 | Uimsbf |
| Current_next_indicator | 1 | Bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 |  |
| real_time_parameters() | 32 | uimsbf |
| for( i=0; i<Nmax; i++ ) { |  |  |
| IFEC_data_byte | 8 | uimsbf |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

## Table 3 — Time Slicing and MPE-IFEC real time parameters
_§5.3, PDF pp. 18-18_

| Syntax | Number of bits | Identifier |
|---|---|---|
| real_time_parameters () { |  |  |
| delta_t | 12 | uimsbf |
| MPE_boundary | 1 | bslbf |
| frame_boundary | 1 | bslbf |
| prev_burst_size | 18 | uimsbf |
| } |  |  |

## Table 4 — Time Slice and FEC identifier descriptor
_§6.2, PDF pp. 19-19_

| Syntax | Number of bits | Identifier |
|---|---|---|
| time_slice_fec_identifier_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| time_slicing | 1 | bslbf |
| mpe_fec | 2 | uimsbf |
| reserved_for_future_use | 2 | bslbf |
| frame_size | 3 | uimsbf |
| max_burst_duration | 8 | uimsbf |
| max_average_rate | 4 | uimsbf |
| time_slice_fec_id | 4 | uimsbf |
| for( i=0; i<id_selector_length; i++ ) { |  |  |
| id_selector_byte | 8 | bslbf |
| } |  |  |
| } |  |  |

## Table 5 — Syntax and semantics for mpe_fec
_§6.2, PDF pp. 20-20_

| value | MPE-FEC | algorithm |
|---|---|---|
| 00 | MPE-FEC not used | n/a |
| 01 | MPE-FEC used | Reed-Solomon (255, 191, 64) |
| 10 | Reserved for future use |  |
| 11 | Reserved for future use |  |

## Table 6 — Syntax and semantics for frame_size
_§6.2, PDF pp. 20-20_

| Size | Max Burst Size | MPE-FEC Frame rows |
|---|---|---|
| 0x00 | 512 kbits = 524 288 bits | 256 |
| 0x01 | 1 024 kbits | 512 |
| 0x02 | 1 536 kbits | 768 |
| 0x03 | 2 048 kbits | 1 024 |
| 0x04 to 0x07 | Reserved for future use | reserved for future use |

## Table 7 — Syntax and semantics for max_average_rate
_§6.2, PDF pp. 21-21_

| Code | Bitrate |
|---|---|
| 0000 | 16 kbps |
| 0001 | 32 kbps |
| 0010 | 64 kbps |
| 0011 | 128 kbps |
| 0100 | 256 kbps |
| 0101 | 512 kbps |
| 0110 | 1 024 kbps |
| 0111 | 2 048 kbps |
| 1000 to 1111 | reserved for future use |

## Table 8 — Syntax and semantics for id_selector_length
_§6.2, PDF pp. 21-21_

| Time_slice_fec_id | Id_selector_length in bytes |
|---|---|
| 0x0 | 0 |
| 0x1 | 9 |
| other | Undefined |

## Table 9 — Semantics for time_slice_fec_id = 0x1
_§6.2, PDF pp. 21-21_

| Syntax | Number of bits | Identifier |
|---|---|---|
| Time_slice_fec_id_0x1() { |  |  |
| T_code | 2 | Uimsbf |
| G_code | 3 | uimsbf |
| Reserved for future use | 3 | bslbf |
| R | 8 | uimsbf |
| C | 13 | uimsbf |
| Reserved for future use | 3 | bslbf |
| B | 8 | uimsbf |
| S | 8 | uimsbf |
| D | 8 | uimsbf |
| EP | 8 | uimsbf |
| Max_rate_averaged_over_B | 8 | uimsbf |
| } |  |  |

## Table 10 — Syntax and semantics for T_code
_§6.2, PDF pp. 22-22_

| Bit rate | Description |
|---|---|
| 00 | Reed Solomon code ([1], clause 9.5.1) |
| 01 | Raptor Codes ([2], clause C.4) |
| 01 to 11 | Reserved for future use |

> **Spec note:** the PDF's Table 10 literally lists the reserved range as `01 to 11`, which textually overlaps the `01` (Raptor) row above it. This is reproduced verbatim from the spec — it is an apparent editorial quirk in EN/TS 102 772 V1.1.1, not a transcription choice. Implementations should treat `01` as Raptor and `10`/`11` as reserved.

## Table 11 — Syntax and semantics for G_code parameter
_§6.2, PDF pp. 22-22_

| Bit rate | G |
|---|---|
| 000 | 1 |
| 001 | 2 |
| 010 | 4 |
| 011 | 8 |
| 100 | 16 |
| 101 | 32 |
| 110 | 64 |
| 111 | 128 |

## Table 12 — Recommended parameter settings for frame_size=0x03
_§6.4.5, PDF pp. 26-26_

|  | EP=1 |  | EP=4 |  | EP=8 |  | EP=16 |  | EP=32 |  |
|---|---|---|---|---|---|---|---|---|---|---|
|  | C | G | C | G | C | G | C | G | C | G |
| r=7/8 | 3584 | 16 | 896 | 4 | 224 | 1 | 448 | 2 | 224 | 1 |
| r=3/4 | 3072 | 16 | 768 | 4 | 192 | 1 | 384 | 2 | 192 | 1 |
| r=2/3 | 2730 | 16 | 682 | 4 | 170 | 1 | 341 | 2 | 170 | 1 |
| r=1/2 | 2048 | 16 | 512 | 4 | 128 | 1 | 256 | 2 | 128 | 1 |
