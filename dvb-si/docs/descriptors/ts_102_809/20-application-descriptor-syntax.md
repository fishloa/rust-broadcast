## Table 20 — Application descriptor syntax
_§5.3.5.3, PDF pp. 38-38_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| application_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x00 |
| descriptor_length | 8 | uimsbf |  |
| application_profiles_length | 8 | uimsbf |  |
| for( i=0; i<N; i++ ) { |  |  |  |
| application_profile | 16 | uimsbf |  |
| version.major | 8 | uimsbf |  |
| version.minor | 8 | uimsbf |  |
| version.micro | 8 | uimsbf |  |
| } |  |  |  |
| service_bound_flag | 1 | bslbf |  |
| visibility | 2 | bslbf |  |
| reserved_future_use | 5 | bslbf |  |
| application_priority | 8 | uimsbf |  |
| for( i=0; i<N; i++ ) { |  |  |  |
| transport_protocol_label | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

