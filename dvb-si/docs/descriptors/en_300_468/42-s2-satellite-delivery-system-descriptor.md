## Table 42 — S2 satellite delivery system descriptor
_§6.2.13.3, PDF pp. 75-75_

| Syntax | Number of bits | Identifier |
|---|---|---|
| S2_satellite_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| scrambling_sequence_selector | 1 | bslbf |
| multiple_input_stream_flag | 1 | bslbf |
| reserved_zero_future_use | 1 | bslbf |
| not_timeslice_flag | 1 | bslbf |
| reserved_future_use | 2 | bslbf |
| TS_GS_mode | 2 | uimsbf |
| if (scrambling_sequence_selector == 0b1) { |
| reserved_future_use | 6 | bslbf |
| scrambling_sequence_index | 18 | uimsbf |
| } |
| if (multiple_input_stream_flag == 0b1) { |
| input_stream_identifier | 8 | uimsbf |
| } |
| if (not_timeslice_flag == 0b0) { |
| timeslice_number | 8 | uimsbf |
| } |
| } |

