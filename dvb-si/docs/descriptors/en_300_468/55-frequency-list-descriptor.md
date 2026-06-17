## Table 55 — Frequency list descriptor
_§6.2.18.1, PDF pp. 80-80_

| Syntax | Number of bits | Identifier |
|---|---|---|
| frequency_list_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| reserved_future_use | 6 | bslbf |
| coding_type | 2 | bslbf |
| for (i=0;i<N;i++) { |
| centre_frequency | 32 | uimsbf |
| } |
| } |

