## Table 160 — Video depth range descriptor
_§6.4.16.1, PDF pp. 147-147_

| Syntax | Number of bits | Identifier |
|---|---|---|
| video_depth_range_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| range_type | 8 | uimsbf |
| range_length | 8 | uimsbf |
| if (range_type == 0x0) { |
| production_disparity_hint_info() |
| } else if (range_type == 0x1) { |
| /* empty */ |
| } else { |
| for (j=0;j<N;j++) { |
| range_selector_byte | 8 | bslbf |
| } |
| } |
| } |
| } |

