## Table 156 — Target region descriptor
_§6.4.12, PDF pp. 143-143_

| Syntax | Number of bits | Identifier |
|---|---|---|
| target_region_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| country_code | 24 | bslbf |
| for (i=0;i<N;i++) { |
| reserved_future_use | 5 | bslbf |
| country_code_flag | 1 | bslbf |
| region_depth | 2 | uimsbf |
| if (country_code_flag == 0b1) { |
| country_code | 24 | bslbf |
| } |
| if (region_depth >= 1) { |
| primary_region_code | 8 | uimsbf |
| if (region_depth >= 2) { |
| secondary_region_code | 8 | uimsbf |
| if (region_depth == 3) { |
| tertiary_region_code | 16 | uimsbf |
| } |
| } |
| } |
| } |
| } |

