## Table 119 — SH delivery system descriptor
_§6.4.6.2, PDF pp. 119-119_

| Syntax | Number of bits | Identifier |
|---|---|---|
| SH_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| diversity_mode | 4 | bslbf |
| reserved_future_use | 4 | bslbf |
| for (i=0;i<N;i++) { |
| modulation_type | 1 | bslbf |
| interleaver_presence | 1 | bslbf |
| interleaver_type | 1 | bslbf |
| reserved_future_use | 5 | bslbf |
| if (modulation_type == 0b0) { |
| polarization | 2 | bslbf |
| roll_off | 2 | bslbf |
| modulation_mode | 2 | bslbf |
| code_rate | 4 | bslbf |
| symbol_rate | 5 | bslbf |
| reserved_future_use | 1 | bslbf |
| } else { |
| bandwidth | 3 | bslbf |
| priority | 1 | bslbf |
| constellation_and_hierarchy | 3 | bslbf |
| code_rate | 4 | bslbf |
| guard_interval | 2 | bslbf |
| transmission_mode | 2 | bslbf |
| common_frequency | 1 | bslbf |
| } |
| if (interleaver_presence == 0b1) { |
| if (interleaver_type == 0b0) { |
| common_multiplier | 6 | uimsbf |
| nof_late_taps | 6 | uimsbf |
| nof_slices | 6 | uimsbf |
| slice_distance | 8 | uimsbf |
| non_late_increments | 6 | uimsbf |
| } else { |
| common_multiplier | 6 | uimsbf |
| reserved_future_use | 2 | uimsbf |
| } |
| } |
| } |
| } |

