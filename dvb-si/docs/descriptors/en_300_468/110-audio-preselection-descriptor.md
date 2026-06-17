## Table 110 — Audio preselection descriptor
_§6.4.1, PDF pp. 114-114_

| Syntax | Number of bits | Identifier |
|---|---|---|
| audio_preselection_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| num_preselections | 5 | uimsbf |
| reserved_zero_future_use | 3 | bslbf |
| for (i=0;i<N;i++) { |
| preselection_id | 5 | uimsbf |
| audio_rendering_indication | 3 | uimsbf |
| audio_description | 1 | bslbf |
| spoken_subtitles | 1 | bslbf |
| dialogue_enhancement | 1 | bslbf |
| interactivity_enabled | 1 | bslbf |
| language_code_present | 1 | bslbf |
| text_label_present | 1 | bslbf |
| multi_stream_info_present | 1 | bslbf |
| future_extension | 1 | bslbf |
| if (language_code_present == 0b1) { |
| ISO_639_language_code | 24 | bslbf |
| } |
| if (text_label_present == 0b1) { |
| message_id | 8 | uimsbf |
| } |
| if (multi_stream_info_present == 0b1) { |
| num_aux_components | 3 | uimsbf |
| reserved_zero_future_use | 5 | bslbf |
| for (j=0;j<N;j++) { |
| component_tag | 8 | uimsbf |
| } |
| } |
| if (future_extension == 0b1) { |
| reserved_zero_future_use | 3 | bslbf |
| future_extension_length | 5 | uimsbf |
| for (j=0;j<N;j++) { |
| future_extension_byte | 8 | uimsbf |
| } |
| } |
| } |
| } |

