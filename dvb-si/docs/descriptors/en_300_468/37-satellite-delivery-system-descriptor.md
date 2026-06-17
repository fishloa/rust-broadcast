## Table 37 — Satellite delivery system descriptor
_§6.2.13.2, PDF pp. 73-73_

| Syntax | Number of bits | Identifier |
|---|---|---|
| satellite_delivery_system_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| frequency | 32 | bslbf |
| orbital_position | 16 | bslbf |
| west_east_flag | 1 | bslbf |
| polarization | 2 | bslbf |
| if (modulation_system == 0b1) { |
| roll_off | 2 | bslbf |
| } else { |
| reserved_zero_future_use | 2 | bslbf |
| } |
| modulation_system | 1 | bslbf |
| modulation_type | 2 | bslbf |
| symbol_rate | 28 | bslbf |
| FEC_inner | 4 | bslbf |
| } |

