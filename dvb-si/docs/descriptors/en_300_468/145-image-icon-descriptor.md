## Table 145 — Image icon descriptor
_§6.4.7, PDF pp. 135-135_

| Syntax | Number of bits | Identifier |
|---|---|---|
| image_icon_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| descriptor_number | 4 | uimsbf |
| last_descriptor_number | 4 | uimsbf |
| reserved_future_use | 5 | uimsbf |
| icon_id | 3 | uimsbf |
| if (descriptor_number == 0x00) { |
| icon_transport_mode | 2 | uimsbf |
| position_flag | 1 | bslbf |
| if (position_flag == 0b1 |
| coordinate_system | 3 | uimsbf |
| reserved_future_use | 2 | bslbf |
| icon_horizontal_origin | 12 | uimsbf |
| icon_vertical_origin | 12 | uimsbf |
| } else { |
| reserved_future_use | 5 | bslbf |
| } |
| icon_type_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| icon_type_char | 8 | uimsbf |
| } |
| if (icon_transport_mode == 0) { |
| icon_data_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| icon_data_byte | 8 | uimsbf |
| } |
| } else if (icon_transport_mode == 1) { |
| url_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| url_char | 8 | uimsbf |
| } |
| } |
| } else { |
| icon_data_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| icon_data_byte | 8 | uimsbf |
| } |
| } |
| } |

