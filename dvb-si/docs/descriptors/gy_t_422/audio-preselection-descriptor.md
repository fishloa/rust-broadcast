## GY/T 422-2025 Table 1 - Audio Preselection Descriptor (multi-audio)
_GY/T 422-2025 电视节目多音频传输技术规范, clause 6, Table 1, printed pp. 2-3 (verified on the rendered pages). Chinese original, free NRTA download. Placed in the PMT ES_info loop of each audio stream (only in the loop of the ES that carries the main data when one audio spans several ES). descriptor_tag 0x7F, descriptor_tag_extension 0x19._

| Syntax | Bits | Mnemonic |
|---|---|---|
| audio_preselection_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| num_preselections | 5 | uimsbf |
| reserved_zero_future_use | 3 | bslbf |
| for (i=0; i<N; i++) { | | |
| preselection_id | 5 | uimsbf |
| audio_rendering_indication | 3 | uimsbf |
| reserved_zero_future_use | 4 | bslbf |
| language_code_present | 1 | bslbf |
| reserved_zero_future_use | 1 | bslbf |
| multi_stream_info_present | 1 | (blank in source; bslbf) |
| future_extension | 1 | bslbf |
| if (language_code_present == 0b1) { ISO_639_language_code | 24 | bslbf } | | |
| if (multi_stream_info_present == 0b1) { num_aux_components 3 uimsbf; reserved_zero_future_use 5 bslbf; for (...) { component_tag 8 uimsbf } } | | |
| if (future_extension == 0b1) { reserved_zero_future_use 3 bslbf; future_extension_length 5 uimsbf; for (...) { future_extension_byte 8 uimsbf } } | | |
| } | | |
| } | | |

Relationship to EN 300 468 Table 110 (dvb-si `AudioPreselection`): identical layout, with the DVB bits `audio_description`, `spoken_subtitles`, `dialogue_enhancement`, `interactivity_enabled` (the "reserved_zero_future_use (4)") and `text_label_present` (the "reserved_zero_future_use (1)") defined as reserved zero in GY/T 422. `num_aux_components` shall not be 0.

### Table 2 audio_rendering_indication
0 no channel-presentation requirement; 1 stereo (two channels); 2 surround (5.1); 3 three-dimensional sound (5.1.4); 4 headphone playback selected; 5-7 reserved. (EN 300 468 Table 111: 0 no preference, 1 stereo, 2 two-dimensional e.g. 5.1, 3 three-dimensional, 4 pre-rendered for headphones.)
