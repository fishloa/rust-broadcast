## Table 19 — CRI structure type and structure id
_§7.3.1.3, PDF pp. 35-35_

| cri_structure_type value | cri_structure_id value | Description |
|---|---|---|
| 0x00 | not defined | reserved |
| 0x01 | 0x00 | results_list |
| 0x02 | 0x00 | data_repository |
| 0x03 | not defined | reserved |
| 0x04 | 0x00 to 0xFF | cri_index |
| 0x05 | 0x00 to 0xFF | cri_prepend_index or cri_leaf_index |
| 0x06 to 0x07 | not defined | reserved |
| 0x08 | 0x00 | result_data |
| 0x09 | 0x00 | services |
| 0x0A to 0xEF | not defined | DVB Reserved |
| 0xF0 to 0xFF | not defined | User Private |

