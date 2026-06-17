## Table 153 — Supplementary audio descriptor
_§6.4.11, PDF pp. 141-141_

| Syntax | Number of bits | Identifier |
|---|---|---|
| supplementary_audio_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| mix_type | 1 | uimsbf |
| editorial_classification | 5 | uimsbf |
| reserved_future_use | 1 | bslbf |
| language_code_present | 1 | uimsbf |
| if (language_code_present == 0b1) { |
| ISO_639_language_code | 24 | bslbf |
| } |
| for (i=0;i<N;i++) { |
| private_data_byte | 8 | uimsbf |
| } |
| } |

