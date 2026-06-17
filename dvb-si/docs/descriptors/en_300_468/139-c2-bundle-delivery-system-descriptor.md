## Table 139 — C2 bundle delivery system descriptor
_§6.4.6.4, PDF pp. 126-126_

| Syntax | Number of bits | Identifier |
|---|---|---|
| C2_bundle_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| plp_id | 8 | uimsbf |
| data_slice_id | 8 | uimsbf |
| C2_System_tuning_frequency | 32 | bslbf |
| C2_System_tuning_frequency_type | 2 | uimsbf |
| active_OFDM_symbol_duration | 3 | bslbf |
| guard_interval | 3 | bslbf |
| primary_channel | 1 | bslbf |
| reserved_zero_future_use | 7 | bslbf |
| } |
| } |

