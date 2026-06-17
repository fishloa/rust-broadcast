## Table 94 — Short smoothing buffer descriptor
_§6.2.38, PDF pp. 103-103_

| Syntax | Number of bits | Identifier |
|---|---|---|
| short_smoothing_buffer_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| sb_size | 2 | uimsbf |
| sb_leak_rate | 6 | uimsbf |
| for (i=0;i<N;i++) { |
| reserved_future_use | 8 | bslbf |
| } |
| } |

