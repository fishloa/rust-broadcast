## Table 162a — VVC subpictures descriptor
_§6.4.17, PDF pp. 149-149_

| Syntax | Number of bits | Identifier |
|---|---|---|
| vvc_subpictures_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| default_service_mode | 1 | bslbf |
| service_description_present | 1 | bslbf |
| number_of_vvc_subpictures | 6 | uimsbf |
| for (i=0;i<N;i++) { |
| component_tag | 8 | uimsbf |
| vvc_subpicture_id | 8 | uimsbf |
| } |
| reserved_zero_future_use | 5 | bslbf |
| processing_mode | 3 | bslbf |
| if (service_description_present == 0b1) { |
| service_description_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| char | 8 | bslbf |
| } |
| } |
| } |

