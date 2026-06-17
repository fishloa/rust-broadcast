## Table 13 — Syntax of the IP/MAC_notification_section
_§8.4.4.1, PDF pp. 28-28_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| IP/MAC_notification_section () { |  |  |  |
| table_id | 8 | uimsbf | 0x4C |
| section_syntax_indicator | 1 | bslbf | 1b |
| reserved_for_future_use | 1 | bslbf | 1b |
| reserved | 2 | bslbf | 11b |
| section_length | 12 | uimsbf |  |
| action_type | 8 | uimsbf | see Table 14 |
| platform_id_hash | 8 | uimsbf |  |
| reserved | 2 | bslbf | 11b |
| version_number | 5 | uimsbf |  |
| current_next_indicator | 1 | bslbf | 1b |
| section_number | 8 | uimsbf |  |
| last_section_number | 8 | uimsbf |  |
| platform_id | 24 | uimsbf |  |
| processing_order | 8 | uimsbf | 0x00 |
| platform_descriptor_loop() |  |  |  |
| for (i=0, i<N1, i++) { |  |  |  |
| target_descriptor_loop() |  |  |  |
| operational_descriptor_loop() |  |  |  |
| } |  |  |  |
| CRC_32 | 32 | rpchof |  |
| } |  |  |  |

