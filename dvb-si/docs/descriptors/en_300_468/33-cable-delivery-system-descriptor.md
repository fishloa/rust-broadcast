## Table 33 — Cable delivery system descriptor
_§6.2.13.1, PDF pp. 72-72_

| Syntax | Number of bits | Identifier |
|---|---|---|
| cable_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| frequency | 32 | bslbf |
| reserved_future_use | 12 | bslbf |
| FEC_outer | 4 | bslbf |
| modulation | 8 | bslbf |
| symbol_rate | 28 | bslbf |
| FEC_inner | 4 | bslbf |
| } |

