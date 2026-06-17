## Table 11 — Syntax of the Update Notification Section
_§9.4.2.0, PDF pp. 22-22_

| Syntax | No. of | Identifier | Default value / remark |
|---|---|---|---|
| | bits | | |
| Update_Notification_Table() { | | | |
| table_id | 8 | uimsbf | 0x4B |
| section_syntax_indicator | 1 | bslbf | 1 |
| reserved_for_future_use | 1 | bslbf | 1 |
| reserved | 2 | bslbf | 11 |
| section_length | 12 | uimsbf | maximum value is 0xFFD |
| action_type | 8 | uimsbf | 0x01 |
| OUI_hash | 8 | uimsbf | |
| reserved | 2 | bslbf | 11 |
| version_number | 5 | uimsbf | |
| current_next_indicator | 1 | bslbf | 1 |
| section_number | 8 | uimsbf | |
| last_section_number | 8 | uimsbf | |
| OUI | 24 | uimsbf | |
| processing_order | 8 | uimsbf | |
| common_descriptor_loop() | variable | | see clause 9.4.2.1 |
| for (i=0; i<N; i++) { | | | |
| compatibilityDescriptor() | variable | | see clause 9.4.2.2 |
| platform_loop_length | 16 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| target_descriptor_loop() | variable | | see clause 9.4.2.3 |
| operational_descriptor_loop() | variable | | see clause 9.4.2.4 |
| } | | | |
| } | | | |
| CRC_32 | 32 | rpchof | |
| } | | | |

