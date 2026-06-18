## Table 2-100 — MVC extension descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.78, Table 2-100; PDF p.112._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MVC_extension_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;average_bit_rate | 16 | uimsbf |
| &nbsp;&nbsp;maximum_bitrate | 16 | uimsbf |
| &nbsp;&nbsp;view_association_not_present | 1 | bslbf |
| &nbsp;&nbsp;base_view_is_left_eyeview | 1 | bslbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;view_order_index_min | 10 | bslbf |
| &nbsp;&nbsp;view_order_index_max | 10 | bslbf |
| &nbsp;&nbsp;temporal_id_start | 3 | bslbf |
| &nbsp;&nbsp;temporal_id_end | 3 | bslbf |
| &nbsp;&nbsp;no_sei_nal_unit_present | 1 | bslbf |
| &nbsp;&nbsp;no_prefix_nal_unit_present | 1 | bslbf |
| } |  |  |
