## Table 106 — VBI data descriptor
_§6.2.47, PDF pp. 110-110_

| Syntax | Number of bits | Identifier |
|---|---|---|
| VBI_data_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| data_service_id | 8 | uimsbf |
| data_service_descriptor_length | 8 | uimsbf |
| if (data_service_id == 0x01 \|\| data_service_id == 0x02 \|\| data_service_id == 0x04 \|\| data_service_id == 0x05 \|\| data_service_id == 0x06 \|\| data_service_id == 0x07) { |
| for (j=0;j<N;j++) { |
| reserved_future_use | 2 | bslbf |
| field_parity | 1 | bslbf |
| line_offset | 5 | uimsbf |
| } |
| } else { |
| for (j=0;j<N;j++) { |
| reserved_future_use | 8 | bslbf |
| } |
| } |
| } |
| } |

