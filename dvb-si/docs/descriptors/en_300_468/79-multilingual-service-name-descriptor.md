## Table 79 — Multilingual service name descriptor
_§6.2.26, PDF pp. 95-95_

| Syntax | Number of bits | Identifier |
|---|---|---|
| multilingual_service_name_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ISO_639_language_code | 24 | bslbf |
| service_provider_name_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| char | 8 | uimsbf |
| } |
| service_name_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| char | 8 | uimsbf |
| } |
| } |
| } |

