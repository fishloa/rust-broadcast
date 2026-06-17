## Table 157 — Target region name descriptor
_§6.4.13, PDF pp. 144-144_

| Syntax | Number of bits | Identifier |
|---|---|---|
| target_region_name_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| country_code | 24 | bslbf |
| ISO_639_language_code | 24 | bslbf |
| for (i=0;i<N;i++) { |
| region_depth | 2 | uimsbf |
| name_length | 6 | uimsbf |
| for (j=0;j<N;j++) { |
| char | 8 | uimsbf |
| } |
| primary_region_code | 8 | uimsbf |
| if (region_depth >= 2) { |
| secondary_region_code | 8 | uimsbf |
| if (region_depth == 3) { |
| tertiary_region_code | 16 | uimsbf |
| } |
| } |
| } |
| } |

