## Table 89 — DVB-HTML application boundary descriptor syntax
_§10.10.3, PDF pp. 174-174_

| No. of Bits | Identifier | Value |
|---|---|---|
| dvb_html_application_boundary_descriptor { |  |  |
| descriptor_tag | 8 | uimsbf |  |
| descriptor_length | 8 | uimsbf |  |
| label_length | 8 | uimsbf | N1 |
| for(i=0; i<N1; i++) { |  |  |  |
| label_bytes | 8 | uimsbf |  |
| } |  |  |  |
| for(i=0; i<N; i++) { |  |  |  |
| regular_expression_bytes | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

descriptor_tag = 0x0A.
