## Table 8 — Graphics constraints descriptor syntax
_§5.2.10.2, PDF pp. 25-25_

|  | No. of bits | Identifier | Value |
|---|---|---|---|
| graphics_constraints_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x14 |
| descriptor_length | 8 | uimsbf |  |
| reserved_future_use | 5 | bslbf |  |
| can_run_without_visible_ui | 1 | bslbf |  |
| handles_configuration_changed | 1 | bslbf |  |
| handles_externally_controlled_video | 1 | bslbf |  |
| for(i=0;i<N;i++) { |  |  |  |
| graphics_configuration_byte | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

