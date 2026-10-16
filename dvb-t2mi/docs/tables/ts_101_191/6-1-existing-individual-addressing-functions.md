## ETSI TS 101 191 V1.4.1 clause 6.1 — existing individual addressing functions
_Referenced by ETSI TS 102 773 V1.4.1 §5.2.8.1 / Table 5 (PDF p.22): "The format of the individual addressing functions is in accordance with clause 6.1 of ETSI TS 101 191 [3]."_

Each table below was transcribed from, and **verified on the rendered page** of,
`private/specs/etsi_ts_101_191_v01.04.01_dvb_mega_frame_sfn_sync.pdf`
(`pdftoppm -r 80`, looked at; the extracted markdown was not trusted alone).

### Tag mapping (TS 101 191 Table 6, PDF p.12 = TS 102 773 Table 5, PDF p.22)

Both rendered pages were read; the tag values are identical, so the mapping is verified, not assumed.

| Function | function_tag | Applicable to DVB-T2 (TS 102 773 Table 5) |
|---|---|---|
| tx_time_offset_function | 0x00 | yes |
| tx_frequency_offset_function | 0x01 | yes |
| tx_power_function | 0x02 | yes |
| private_data_function | 0x03 | yes |
| cell_id_function | 0x04 | yes |
| enable_function | 0x05 | yes |
| bandwidth_function | 0x06 | no |
| Future_use | 0x07..=0xFF | — |

> **Spec note (function_length):** TS 101 191 §6.1 says "The function_length field gives the total length of the function field in bytes" and TS 102 773 §5.2.8.2.0 (PDF p.22) says it "defines the total length of the function() in bytes, **including the function_tag, function_length and function_body() fields**", with the NOTE that for the existing TS 101 191 functions `function_body()` "comprises all the fields that follow the function_length field". The real capture `fixtures/dvb-t2mi/colombia-capital-t2mi.ts` confirms it: `00 04 ff 9c` is a time-offset function with a 2-byte body, so `function_length` = 4 = 2 + body length. `function_length` therefore counts the 2-byte header, not just the body.

### Table 7 — Function transmitter time offset (§6.1.1, PDF p.12)

| Syntax | Number of bits | Identifier |
|---|---|---|
| tx_time_offset_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| time_offset | 16 | tcimsbf |
| } | | |

`time_offset`: unit 100 ns, range [-32 768, 32 767] x 100 ns (two's complement).

### Table 8 — Function transmitter frequency offset (§6.1.2, PDF p.12)

| Syntax | Number of bits | Identifier |
|---|---|---|
| tx_frequency_offset_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| frequency_offset | 24 | tcimsbf |
| } | | |

`frequency_offset`: unit 1 Hz, range [-8 388 608, 8 388 607] Hz (two's complement).

### Table 9 — Function transmitter power (§6.1.3, PDF p.13)

| Syntax | Number of bits | Identifier |
|---|---|---|
| tx_power_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| tx_power | 16 | uimsbf |
| } | | |

`tx_power`: ERP, unit 0,1 dB, range ([0, 65535] x 0,1) dBm.

### Table 10 — Function private data (§6.1.4, PDF p.13)

| Syntax | Number of bits | Identifier |
|---|---|---|
| private_data_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| for (i=0; i<N; i++) { | | |
| private_data | 8 | bsblf |
| } | | |
| } | | |

`private_data`: opaque proprietary bytes (no structure is defined by the spec).

### Table 11 — Function cell id (§6.1.5, PDF p.13)

| Syntax | Number of bits | Identifier |
|---|---|---|
| cell_id_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| cell_id | 16 | uimsbf |
| wait_for_enable_flag | 1 | bsblf |
| reserved_future_use | 7 | bsblf |
| } | | |

`wait_for_enable_flag` 0: insert cell_id immediately; 1: insert after the corresponding enable_function.

### Table 12 — Function enable (§6.1.6, PDF p.14)

| Syntax | Number of bits | Identifier |
|---|---|---|
| enable_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| for (i=0; i<N; i++) { | | |
| enabled_function_tag | 8 | bsblf |
| } | | |
| } | | |

`enabled_function_tag`: "The coding is according to table 6" (the function_tag values above).

### Table 13 — Function bandwidth (§6.1.7, PDF p.14)

| Syntax | Number of bits | Identifier |
|---|---|---|
| bandwidth_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| ch_bandwidth | 7 | uimsbf |
| wait_for_enable_flag | 1 | bsblf |
| } | | |

> **Spec note (ch_bandwidth width):** the prose under Table 13 says "This 8-bit field indicates the DVB-T channel bandwidth", but Table 13 allots 7 bits (and 1 to `wait_for_enable_flag`), and Table 14 only defines values 0..=127. The 7-bit syntax-table width is implemented; the "8-bit" prose is not silently corrected, it is recorded here.

### Table 14 — Coding of channel bandwidth (PDF p.14)

| bandwidth | Channel bandwidth |
|---|---|
| 0 | 5 MHz |
| 1 to 127 | Reserved for future use |

`wait_for_enable_flag` 0: implement the bandwidth immediately; 1: after the corresponding enable_function. TS 102 773 Table 5 marks this function "not applicable to DVB-T2"; it is still parsed (a gateway may emit it) but carries no T2 meaning.
