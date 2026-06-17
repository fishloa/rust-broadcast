## Table 22 — Syntax of the DFIS
_§5.3.2.3.1, PDF pp. 30-31_

| Syntax | Number of bits | Identifier |
|---|---|---|
| downloadable_font_information_section() { | | |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| reserved_future_use | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| font_id_extension | 9 | bslbf |
| font_id | 7 | uimsbf |
| reserved | 2 | bslbf |
| version_number | 5 | uimsbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimbsf |
| last_section_number | 8 | uimbsf |
| for (i=0; i<n; i++) { | | |
| font_info_type | 8 | uimbsf |
| if (font_info_type == 0x00) { | | |
| font_style | 3 | uimsbf |
| font_weight | 4 | uimsbf |
| reserved_zero_future_use | 1 | bslbf |
| } | | |
| if (font_info_type == 0x01) { | | |
| reserved_zero_future_use | 4 | bslbf |
| font_file_format | 4 | uimsbf |
| uri_length | 8 | uimsbf |
| for (j=0; j<n; j++){ | | |
| uri_char | 8 | bslbf |
| } | | |
| } | | |
| if (font_info_type == 0x02) { | | |
| font_size | 16 | uimsbf |
| } | | |
| if (font_info_type >= 0x02) { | | |
| font_info_length | 8 | uimsbf |
| for (j=0; j<n; j++){ | | |
| text_char | 8 | uimsbf |
| } | | |
| } | | |
| } | | |
| CRC_32 | 32 | rpchof |
| } | | |

