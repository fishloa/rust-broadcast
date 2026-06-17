## Table 140 — S2X satellite delivery system descriptor
_§6.4.6.5.2, PDF pp. 128-128_

| Syntax | Number of bits | Identifier |
|---|---|---|
| S2X_satellite_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| receiver_profiles | 5 | bslbf |
| reserved_zero_future_use | 3 | bslbf |
| S2X_mode | 2 | uimsbf |
| scrambling_sequence_selector | 1 | bslbf |
| reserved_zero_future_use | 3 | bslbf |
| TS_GS_S2X_mode | 2 | bslbf |
| if (scrambling_sequence_selector == 0b1) { |
| reserved_zero_future_use | 6 | bslbf |
| scrambling_sequence_index | 18 | uimsbf |
| } |
| frequency (see note) | 32 | bslbf |
| orbital_position (see note) | 16 | bslbf |
| west_east_flag (see note) | 1 | bslbf |
| polarization (see note) | 2 | bslbf |
| multiple_input_stream_flag (see note) | 1 | bslbf |
| reserved_zero_future_use | 1 | bslbf |
| roll_off (see note) | 3 | bslbf |
| reserved_zero_future_use | 4 | bslbf |
| symbol_rate (see note) | 28 | bslbf |
| if (multiple_input_stream_flag == 0b1) { |
| input_stream_identifier (see note) | 8 | uimsbf |
| } |
| if (S2X_mode == 2) { |
| timeslice_number | 8 | uimsbf |
| } |
| if (S2X_mode == 3) { |
| reserved_zero_future_use | 7 | bslbf |
| num_channel_bonds_minus_one | 1 | uimsbf |
| for (i=0;i<N;i++) { |
| frequency | 32 | bslbf |
| orbital_position | 16 | bslbf |
| west_east_flag | 1 | bslbf |
| polarization | 2 | bslbf |
| bonded_channel_multiple_input_stream_flag | 1 | bslbf |
| reserved_zero_future_use | 1 | bslbf |
| roll_off | 3 | bslbf |
| reserved_zero_future_use | 4 | bslbf |
| symbol_rate | 28 | bslbf |
| if (bonded_channel_multiple_input_stream_flag == 0b1) { |
| input_stream_identifier | 8 | uimsbf |
| } |
| } |
| } |
| for (i=0;i<N;i++) { |
| reserved_future_use | 8 | bslbf |
| } |
| } |
| NOTE: When channel bonding is used, these parameters describe the primary | channel. |

