## Table 105 — Transport stream descriptor
_§6.2.46, PDF pp. 109-109_

| Syntax | Number of bits | Identifier |
|---|---|---|
| transport_stream_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| byte | 8 | uimsbf |
| } |
| } |

