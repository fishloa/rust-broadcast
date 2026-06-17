## Table 36 — Syntax of extended data broadcast descriptor - broadcast id 0xF2
_§5.3.9.1, PDF pp. 49-49_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| data_broadcast_descriptor(){ |  |  |  |
| descriptor_tag | 8 | uimsbf |  |
| descriptor_length | 8 | uimsbf |  |
| data_broadcast_id | 16 | uimsbf |  |
| component_tag | 8 | uimsbf |  |
| selector_length | 8 | uimsbf |  |
| for(i=0; i<selector_length; i++){ |  |  |  |
| organization_id | 32 | uimsbf |  |
| application_id | 16 | uimsbf |  |
| reserved_future_use | 1 | bslbf |  |
| application_type | 15 | uimsbf |  |
| application_profile_length | 8 | uimsbf |  |
| for (j=0; j<N; j++){ |  |  |  |
| application_profile | 16 | uimsbf |  |
| version.major | 8 | uimsbf |  |
| version.minor | 8 | uimsbf |  |
| version.micro | 8 | uimsbf |  |
| } |  |  |  |
| application_names_length | 8 | uimsbf |  |
| for(j=0; j<N2;j++){ |  |  |  |
| ISO_639_language_code | 24 | bslbf |  |
| application_name_length | 8 | uimsbf |  |
| for(l=0; l<N3; l++){ |  |  |  |
| application_name_char | 8 | bslbf |  |
| } |  |  |  |
| } |  |  |  |
| reserved_length | 8 | uimsbf |  |
| for(j=0; i<N4; i++){ |  |  |  |
| reserved_future_use | 8 | bslbf |  |
| } |  |  |  |
| private_data_length | 8 | uimsbf |  |
| for(j=0; j<N5; j++){ |  |  |  |
| private_data_byte | 8 | bslbf |  |
| } |  |  |  |
| } |  |  |  |
| ISO_639_language_code | 24 | bslbf |  |
| text_length | 8 | uimsbf |  |
| for (i=0; i<text_length; i++){ |  |  |  |
| text_char | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

