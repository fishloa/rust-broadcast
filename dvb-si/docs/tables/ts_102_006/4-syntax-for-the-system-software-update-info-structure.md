## Table 4 — Syntax for the system_software_update_info structure
_§7.1, PDF pp. 13-13_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| system_software_update_info() { | | |
| OUI_data_length | 8 | uimsbf |
| for (i=0; i<N; i++) { | | |
| OUI | 24 | bslbf |
| reserved | 4 | bslbf |
| update_type | 4 | uimsbf |
| reserved | 2 | bslbf |
| update_versioning_flag | 1 | uimsbf |
| update_version | 5 | uimsbf |
| selector_length | 8 | uimsbf |
| for (i=0; i<N; i++){ | | |
| selector_byte | 8 | uimsbf |
| } | | |
| } | | |
| for (i=0; i<N; i++){ | | |
| private_data_byte | 8 | uimsbf |
| } | | |
| } | | |

