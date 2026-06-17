## Table 144b — S2Xv2 satellite delivery system info
_§6.4.6.5.3, PDF pp. 131-131_

| Syntax | Number of bits | Identifier |
|---|---|---|
| S2Xv2_satellite_delivery_system_info() { |
| delivery_system_id | 32 | uimsbf |
| S2Xv2_mode | 4 | uimsbf |
| multiple_input_stream_flag | 1 | bslbf |
| roll_off | 3 | bslbf |
| reserved_zero_future_use | 2 | bslbf |
| NCR_reference | 1 | bslbf |
| NCR_version | 1 | bslbf |
| channel_bond | 2 | uimsbf |
| polarization | 2 | bsblf |
| if (S2Xv2_mode == 1 or S2Xv2_mode == 2) { |
| scrambling_sequence_selector | 1 | bslbf |
| } else { |
| reserved_zero_future_use | 1 | bslbf |
| } |
| TS_GS_S2X_mode | 2 | bslbf |
| receiver_profiles | 5 | bslbf |
| satellite_id | 24 | uimsbf |
| frequency | 32 | bslbf |
| symbol_rate | 32 | bslbf |
| if (multiple_input_stream_flag == 1) { |
| input_stream_identifier | 8 | uimsbf |
| } |
| if (S2Xv2_mode == 1 or S2Xv2_mode == 2) { |
| if (scrambling_sequence_selector == 1) { |
| reserved_zero_future_use | 6 | bslbf |
| scrambling_sequence_index | 18 | uimsbf |
| } |
| } |
| if (S2Xv2_mode == 2 or S2Xv2_mode == 5) { |
| timeslice_number | 8 | uimsbf |
| } |
| if (channel_bond == 1) { |
| reserved_zero_future_use | 7 | bslbf |
| num_channel_bonds_minus_one | 1 | uimsbf |
| for (i=0;i<N;i++) { |
| secondary_delivery_system_id | 32 | uimsbf |
| } |
| } |
| if (S2Xv2_mode == 4 or S2Xv2_mode == 5) { |
| SOSF_WH_sequence_number | 8 | uimsbf |
| SFFI_selector | 1 | bsblf |
| beam_hopping_time_plan_selector | 1 | bsblf |
| reserved_zero_future_use | 2 | bsblf |
| reference_scrambing_index | 20 | uimsbf |
| if (SFFI_selector == 1) { |
| SFFI | 4 | bslf |
| } else { |
| reserved_zero_future_use | 4 | bsblf |
| } |
| payload_scrambling_index | 20 | uimsbf |
| if (beam_hopping_time_plan_selector == 1) { |
| beamhopping_time_plan_id | 32 | uimsbf |
| } |
| superframe_pilots_WH_sequence_number | 5 | uimsbf |
| postamble_PLI | 3 | bslf |
| } |
| for (i=0;i<N;i++) { |
| reserved_zero_future_use | 8 | bslbf |
| } |
| } |

