## Table 2-33 — TS program map section
_§2.4.4.8, PDF pp. 58-59_

| Syntax | Bits | Mnemonic |
|---|---|---|
| TS_program_map_section() { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| '0' | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| program_number | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| reserved | 3 | bslbf |
| PCR_PID | 13 | uimsbf |
| reserved | 4 | bslbf |
| program_info_length | 12 | uimsbf |
| for (i = 0; i < N; i++) { |  |  |
| descriptor() |  |  |
| } |  |  |
| for (i = 0; i < N1; i++) { |  |  |
| stream_type | 8 | uimsbf |
| reserved | 3 | bslbf |
| elementary_PID | 13 | uimsbf |
| reserved | 4 | bslbf |
| ES_info_length | 12 | uimsbf |
| for (i = 0; i < N2; i++) { |  |  |
| descriptor() |  |  |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

Cross-check notes (§2.4.4.9): table_id = 0x02; section_syntax_indicator =
`'1'`; section_length first two bits `'00'`, value shall not exceed
**1021 (0x3FD)**; one program definition per TS_program_map_section (so a
program definition is never longer than **1016 (0x3F8)** bytes);
**section_number and last_section_number shall both be 0x00**; version_number
refers to the definition of a single program (single section), incremented by
1 modulo 32 on change; PCR_PID = **0x1FFF** when no PCR is associated with
the program definition (private streams); program_info_length and
ES_info_length each have their first two bits `'00'`, remaining 10 bits give
the descriptor-loop byte count.

