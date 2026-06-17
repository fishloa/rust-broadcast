## Table 11e — Time association info
_§5.2.11.4, PDF pp. 45-45_

| Syntax | Number of bits | Mnemonic |
|---|---|---|
| time_association_info(){ |
| association_type | 4 | uimsbf |
| if (association_type = 1) { |
| leap59 | 1 | bsblf |
| leap61 | 1 | bsblf |
| pastleap59 | 1 | bsblf |
| pastleap61 | 1 | bsblf |
| } else { |
| reserved_zero_future_use | 4 | bsblf |
| } |
| ncr_base | 33 | uimsbf |
| reserved_zero_future_use | 6 | bsblf |
| ncr_ext | 9 | uimsbf |
| association_timestamp_seconds | 64 | uimsbf |
| association_timestamp_nanoseconds | 32 | uimsbf |
| } |

