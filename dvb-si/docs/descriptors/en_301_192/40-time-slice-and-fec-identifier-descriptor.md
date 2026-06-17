## Table 40 — Time Slice and FEC identifier descriptor
_§9.5.0, PDF pp. 53-53_

| Syntax | No. of bits | Identifier |
|---|---|---|
| time_slice_fec_identifier_descriptor () { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| time_slicing | 1 | bslbf |
| mpe_fec | 2 | uimsbf |
| reserved_for_future_use | 2 | bslbf |
| frame_size | 3 | uimsbf |
| max_burst_duration | 8 | uimsbf |
| max_average_rate | 4 | uimsbf |
| time_slice_fec_id | 4 | uimsbf |
| for( i=0; i<N; i++ ) { |  |  |
| id_selector_byte | 8 | bslbf |
| } |  |  |
| } |  |  |

