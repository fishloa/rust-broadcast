## Table 3 — Syntax of datagram_section
_§7.1, PDF pp. 17-17_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| datagram_section() { |  |  |
| table_id | 8 | uimsbf |
| section_syntax_indicator | 1 | bslbf |
| private_indicator | 1 | bslbf |
| reserved | 2 | bslbf |
| section_length | 12 | uimsbf |
| MAC_address_6 | 8 | uimsbf |
| MAC_address_5 | 8 | uimsbf |
| reserved | 2 | bslbf |
| payload_scrambling_control | 2 | bslbf |
| address_scrambling_control | 2 | bslbf |
| LLC_SNAP_flag | 1 | bslbf |
| current_next_indicator | 1 | bslbf |
| section_number | 8 | uimsbf |
| last_section_number | 8 | uimsbf |
| MAC_address_4 | 8 | uimsbf |
| MAC_address_3 | 8 | uimsbf |
| MAC_address_2 | 8 | uimsbf |
| MAC_address_1 | 8 | uimsbf |
| if (LLC_SNAP_flag == "1") { |  |  |
| LLC_SNAP() |  |  |
| } else { |  |  |
| for (j=0;j<N1;j++) { |  |  |
| IP_datagram_data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| if (section_number == last_section_number) { |  |  |
| for (j=0;j<N2;j++) { |  |  |
| stuffing_byte | 8 | bslbf |
| } |  |  |
| } |  |  |
| if (section_syntax_indicator =="0") { |  |  |
| checksum | 32 | uimsbf |
| } else { |  |  |
| CRC_32 | 32 | rpchof |
| } |  |  |
| } |  |  |

