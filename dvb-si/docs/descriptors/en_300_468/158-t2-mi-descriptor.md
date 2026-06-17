## Table 158 — T2-MI descriptor
_§6.4.14, PDF pp. 145-145_

| Syntax | Number of bits | Identifier |
|---|---|---|
| T2MI_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| reserved_zero_future_use | 5 | bslbf |
| t2mi_stream_id | 3 | uimsbf |
| reserved_zero_future_use | 5 | bslbf |
| num_t2mi_streams_minus_one | 3 | uimsbf |
| reserved_zero_future_use | 7 | bslbf |
| pcr_iscr_common_clock_flag | 1 | bslbf |
| for (i=0;i<N;i++) { |
| reserved_zero_future_use | 8 | bslbf |
| } |
| } |

