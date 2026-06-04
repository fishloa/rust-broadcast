# ETSI TS 102 772 v1.1.1 — MPE inter-burst FEC (MPE-IFEC)

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

| Parameter | Unit | Category | Description | Signalling | Scoping |  |  |  |  |  |  |  |  |  |  |  |  |  |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| EP | Datagram | Taxonomy | IFEC Encoding Period | Direct via | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | burst |  |  | Time_slice_fec_identifier | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
| D | Datagram | Taxonomy | Datagram burst sending | Direct via | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | burst |  | delay | Time_slice_fec_identifier | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | Table sizing |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| C | columns | Table sizing | Number of ADST columns | Direct via | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  | Time_slice_fec_identifier | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
| K | columns | Table sizing | Number of ADT columns = | Indirect via | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | EP*C | Time_slice_fec_identifier | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
| N | columns | Table sizing | Number of iFDT columns | Indirect via | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | = EP*R*G | Time_slice_fec_identifier | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
| G | columns | Table sizing | Maximum number of iFDT | Direct | Time_slice_fec_ide |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | columns per IFEC section |  | ntifier |  |  |  |  |  |  |  |  |  |  |  |  |  |
| K | datagrambu | Index | continuous burst counter | N/A | Loop |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | rst |  | internal to sender |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| k' | IFEC burst | field | Burst number | N/A | IFEC section |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | iiFFEECC__hheeaaddeerr |  |  |  |  |  |  |  | iiFFEECC__ddaattaa__bbyytteess |  | CCRRCC |  |  |  |  |  |  |  |
|  |  | iiFFEECC__hheeaaddeerr |  |  |  |  |  |  | iiFFEECC__ddaattaa__bbyytteess |  |  | CCRRCC |  |  |  |  |  |  |
|  |  |  | iiFFEECC__hheeaaddeerr |  |  |  |  |  | iiFFEECC__ddaattaa__bbyytteess |  |  |  | CCRRCC |  |  |  |  |  |
|  |  |  |  | iiFFEECC__hheeaaddeerr |  |  |  |  | iiFFEECC__ddaattaa__bbyytteess |  |  |  |  | CCRRCC |  |  |  |  |
|  |  |  |  | iiFFEECC__hheeaaddeerr |  |  |  |  | iiFFEECC__ddaattaa__bbyytteess |  |  |  |  | CCRRCC3322 |  |  |  |  |

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
| for( i=0; I<id_selector_length; i++ ) |  |  |
| { |  |  |
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
_§6.4.5, PDF pp. 26-33_

|  | EP=1 |  | EP=4 |  | EP=8 |  | EP=16 |  | EP=32 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
|  | C | G | C | G | C | G | C | G | C | G |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| r=7/8 | 3584 | 16 | 896 | 4 | 224 | 1 | 448 | 2 | 224 | 1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| r=3/4 | 3072 | 16 | 768 | 4 | 192 | 1 | 384 | 2 | 192 | 1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| r=2/3 | 2730 | 16 | 682 | 4 | 170 | 1 | 341 | 2 | 170 | 1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| r=1/2 | 2048 | 16 | 512 | 4 | 128 | 1 | 256 | 2 | 128 | 1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | Time Slice |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | Burst |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| MPE | data |  |  |  | MPE iFEC |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  | H |  |  | Datagram |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| MPEse | dcatitoan |  |  |  | MPEs ieFcEtiCon |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| MPE | data | MPE-FEC | HMPE |  | iFEC |  |  | Burst |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| section |  |  | H |  | section |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| section |  | Decoding |  |  | section |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | Padding | Burst Number |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | Information | Detection |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| ADST |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  | ADST |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  | FDT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  | ADT M-1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  | M-1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  | ADT 1 | FDT | 1 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  | ADT 0 | FDT 0 |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| Static Parameters | Description |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| EP | IFEC Encoding period |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| B | IFEC Data Interleaving |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| S | IFEC Spread |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| D | Data delay at sender |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| C | Maximum number of data columns per ADST |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| T | Symbol size of code (Number of rows in ADST/ADT/iFDT) |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| G | Maximum number of symbols per MPE-IFEC section |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| M | Number of Concurrent ADT and iFDT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| K | Number of columns in ADT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| N | Number of columns in iFDT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| R | maximum number of IFEC sections in an IFEC burst |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| k | Modulo counter for burst |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| max |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| T | maximum burst duration |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| bmax |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | p |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | MPE-OFEC |  |  | MPE |  |  |  | MPE-OFEC |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | p |  |  | c |  |  |  | n |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | p |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | MPE-OFEC |  |  |  | MPE |  | MPE-OFEC |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | p |  |  |  | c |  | n |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | ΔT |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  | p |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | MPE-OFEC |  | MPE |  |  |  | MPE-OFEC |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | p |  | c |  |  |  | n |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  | rebmun_noitces_tsal |  |  |  |  |  |  |  |  | rebmun_noitces_tsal |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | rebmun_noitces_tsal |  |  |  |  |  |  |  |  | rebmun_noitces_tsal |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  | EPM |  |  |  |  |  |  | EPM |  |  |  |  |  |  |  |  | EPM |  |  |  |  | )0,’k(CEFi |  | )1,’k(CEFi |  |  | )2,’k(CEFi |  | )k,’k(CEFi |
|  | tsoL |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  | EPM |  |  |  |  |  |  |  |  | EPM |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  | EPM |  |  |  |  |  |  |  |  | EPM |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  | ? |  |  |  |  | ? |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  |  | Unreliable |  | (lost | MPE |  |  | or unknown) |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  |  | Padding |  | fromlast_section_number |  |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  |  | Padding |  | from | PREV_BURST_SIZE |  |  |  |  |  |  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |  |  |  |  |  |  |  | Well | received |  | (fromMPE |  |  | header) |  |  |  |  |  |  |  |  |  |
|  |  | Document history |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |
| V1.1.1 | September 2010 | Publication |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |  |

