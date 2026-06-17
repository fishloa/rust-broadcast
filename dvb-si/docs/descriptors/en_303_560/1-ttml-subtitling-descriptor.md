## Table 1 — TTML subtitling descriptor
_§5.2.1.1, PDF pp. 14-14_

| Syntax | Number of bits | Identifier |
|---|---|---|
| TTML_subtitling_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| ISO_639_language_code | 24 | bslbf |
| subtitle_purpose | 6 | uimsbf |
| TTS_suitability | 2 | uimsbf |
| essential_font_usage_flag | 1 | bslbf |
| qualifier_present_flag | 1 | bslbf |
| reserved_zero_future_use | 2 | bslbf |
| dvb_ttml_profile_count | 4 | uimsbf |
| for(i=0;i<N;i++) { | | |
| dvb_ttml_profile | 8 | uimbsf |
| } | | |
| if (qualifier_present_flag == 1){ | | |
| qualifier | 32 | bslbf |
| } | | |
| if (essential_font_usage_flag == 1){ | | |
| font_count | 8 | uimsbf |
| for(i=0; i<font_count; i++){ | | |
| reserved_zero_future_use | 1 | bslbf |
| font_id | 7 | uimsbf |
| } | | |
| } | | |
| text_length | 8 | bslbf |
| for(i=0;i<N;i++) { | | |
| text_char | 8 | bslbf |
| } | | |
| for(i=0;i<N;i++) { | | |
| reserved_zero_future_use | 8 | bslbf |
| } | | |
| } | | |

