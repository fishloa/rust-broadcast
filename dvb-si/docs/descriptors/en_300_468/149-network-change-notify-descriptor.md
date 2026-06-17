## Table 149 — Network change notify descriptor
_§6.4.9, PDF pp. 138-138_

| Syntax | Number of bits | Identifier |
|---|---|---|
| network_change_notify_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| cell_id | 16 | uimsbf |
| loop_length | 8 | uimsbf |
| for (j=0;j<N;j++) { |
| network_change_id | 8 | uimsbf |
| network_change_version | 8 | uimsbf |
| start_time_of_change | 40 | bslbf |
| change_duration | 24 | uimsbf |
| receiver_category | 3 | uimsbf |
| invariant_ts_present | 1 | bslbf |
| change_type | 4 | uimsbf |
| message_id | 8 | uimsbf |
| if (invariant_ts_present == 0b1) { |
| invariant_ts_tsid | 16 | uimsbf |
| invariant_ts_onid | 16 | uimsbf |
| } |
| } |
| } |
| } |

