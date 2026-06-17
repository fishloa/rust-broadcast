## Table 71 — Mosaic descriptor
_§6.2.21, PDF pp. 91-91_

| Syntax | Number of bits | Identifier |
|---|---|---|
| mosaic_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| mosaic_entry_point | 1 | bslbf |
| number_of_horizontal_elementary_cells | 3 | uimsbf |
| reserved_future_use | 1 | bslbf |
| number_of_vertical_elementary_cells | 3 | uimsbf |
| for (i=0;i<N;i++) { |
| logical_cell_id | 6 | uimsbf |
| reserved_future_use | 7 | bslbf |
| logical_cell_presentation_info | 3 | uimsbf |
| elementary_cell_field_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| reserved_future_use | 2 | bslbf |
| elementary_cell_id | 6 | uimsbf |
| } |
| cell_linkage_info | 8 | uimsbf |
| if (cell_linkage_info == 0x01) { |
| bouquet_id | 16 | uimsbf |
| } |
| if (cell_linkage_info == 0x02) { |
| original_network_id | 16 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| } |
| if (cell_linkage_info == 0x03) { |
| original_network_id | 16 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| } |
| if (cell_linkage_info == 0x04) { |
| original_network_id | 16 | uimsbf |
| transport_stream_id | 16 | uimsbf |
| service_id | 16 | uimsbf |
| event_id | 16 | uimsbf |
| } |
| } |
| } |

