## Table 51 — Syntax of the certificate collection message section
_§9.5.4.9, PDF pp. 91-91_

| Syntax | Number of bits | Identifier |
|---|---|---|
| certificate_collection_message_section() { | | |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| trust_message_id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| reserved | 4 | bslbf |
| certificate_count | 4 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| reserved | 4 | bslbf |
| certificate_length | 12 | uimsbf |
| for (j=0;j<M;j++) { |  |  |
| certificate_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |

