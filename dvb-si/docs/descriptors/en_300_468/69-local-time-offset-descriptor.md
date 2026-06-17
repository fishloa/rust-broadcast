## Table 69 — Local time offset descriptor
_§6.2.20, PDF pp. 89-89_

| Syntax | Number of bits | Identifier |
|---|---|---|
| local_time_offset_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| country_code | 24 | bslbf |
| country_region_id | 6 | bslbf |
| reserved_future_use | 1 | bslbf |
| local_time_offset_polarity | 1 | bslbf |
| local_time_offset | 16 | bslbf |
| time_of_change | 40 | bslbf |
| next_time_offset | 16 | bslbf |
| } |
| } |

