## Table 115 — C2 delivery system descriptor
_§6.4.6.1, PDF pp. 117-117_

| Syntax | Number of bits | Identifier |
|---|---|---|
| C2_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| plp_id | 8 | uimsbf |
| data_slice_id | 8 | uimsbf |
| C2_System_tuning_frequency | 32 | bslbf |
| C2_System_tuning_frequency_type | 2 | uimsbf |
| active_OFDM_symbol_duration | 3 | bslbf |
| guard_interval | 3 | bslbf |
| } |

