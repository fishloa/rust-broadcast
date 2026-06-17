## Table 42 — Syntax of the authentication message section
_§9.4.3, PDF pp. 70-70_

| Syntax | Number of bits | Identifier |
|---|---|---|
| authentication_message_section() { | | |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| authentication_group_id | 16 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| section_hash_algorithm_identifier | 8 | uimsbf |
| section_hash_length | 8 | uimsbf |
| signature_algorithm_identifier | 8 | uimsbf |
| reserved | 4 | bslbf |
| section_hashes_loop_length | 12 | uimsbf |
| for (i=0;i<N;i++) { | | |
| reference_type | 4 | uimsbf |
| reference_length | 4 | uimsbf |
| for (j=0;j<N;j++) { | | |
| reference_byte | 8 | uimsbf |
| } | | |
| for (j=0;j<N;j++) { | | |
| section_hash_byte | 8 | bslbf |
| } | | |
| } | | |
| extension_bytes_length | 8 | uimsbf |
| for (i=0;i<N;i++) { | | |
| extension_byte | 8 | bslbf |
| } | | |
| signature_key_identifier_length | 8 | uimsbf |
| for (i=0;i<N;i++) { | | |
| signature_key_identifier_byte | 8 | bslbf |
| } | | |
| for (i=0;i<N;i++) { | | |
| signature_byte | 8 | bslbf |
| } | | |
| CRC_32 | 32 | rpchof |
| } | | | |

