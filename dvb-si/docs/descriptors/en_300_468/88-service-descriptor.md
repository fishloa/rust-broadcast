## Table 88 — Service descriptor
_§6.2.33, PDF pp. 99-99_

| Syntax | Number of bits | Identifier |
|---|---|---|
| service_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| service_type | 8 | uimsbf |
| service_provider_name_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| service_name_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| char | 8 | uimsbf |
| } |
| } |

