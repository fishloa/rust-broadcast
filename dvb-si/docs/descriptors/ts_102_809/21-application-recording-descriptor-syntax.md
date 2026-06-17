## Table 21 — Application recording descriptor syntax
_§5.3.5.4, PDF pp. 40-40_

| Syntax | No. of bits | Identifier | Comments/Value |
|---|---|---|---|
| application_recording_descriptor (){ |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x06 |
| descriptor_length | 8 | uimsbf |  |
| scheduled_recording_flag | 1 | bslbf |  |
| trick_mode_aware_flag | 1 | bslbf |  |
| time_shift_flag | 1 | bslbf |  |
| dynamic_flag | 1 | bslbf |  |
| av_synced_flag | 1 | bslbf |  |
| initiating_replay_flag | 1 | bslbf |  |
| reserved | 2 | bslbf |  |
| label_count | 8 | uimsbf | N0 |
| for(i=0;i<N0;i++){ |  |  |  |
| label_length | 8 | uimsbf | N1 |
| for(j=0; j<N1; j++) { |  |  |  |
| label_char | 8 | uimsbf |  |
| } |  |  |  |
| storage_properties | 2 | uimsbf |  |
| reserved | 6 |  |  |
| } |  |  |  |
| component_tag_list_length | 8 | uimsbf | N2 |
| for(i=0;i<N2;i++){ |  |  |  |
| component_tag | 8 | uimsbf |  |
| } |  |  |  |
| private_length | 8 | uimsbf | N3 |
| for(i=0;i<N3;i++){ |  |  |  |
| private | 8 | uimsbf |  |
| } |  |  |  |
| for(i=0;i<N4;i++){ |  |  |  |
| reserved_future_use | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

