## Table 2-49 — Hierarchy descriptor
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.6, Table 2-49; PDF p.80._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| hierarchy_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;no_view_scalability_flag | 1 | bslbf |
| &nbsp;&nbsp;no_temporal_scalability_flag | 1 | bslbf |
| &nbsp;&nbsp;no_spatial_scalability_flag | 1 | bslbf |
| &nbsp;&nbsp;no_quality_scalability_flag | 1 | bslbf |
| &nbsp;&nbsp;hierarchy_type | 4 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;hierarchy_layer_index | 6 | uimsbf |
| &nbsp;&nbsp;tref_present_flag | 1 | bslbf |
| &nbsp;&nbsp;reserved | 1 | bslbf |
| &nbsp;&nbsp;hierarchy_embedded_layer_index | 6 | uimsbf |
| &nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;hierarchy_channel | 6 | uimsbf |
| } |  |  |
