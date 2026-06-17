## Table 1 — Syntax for the private data bytes for linkage type 0x09
_§6.1.0, PDF pp. 10-10_

| Syntax | No. of | Identifier |
|---|---|---|
| | bits | |
| system_software_update_link_structure(){ | | |
| OUI_data_length | 8 | uimsbf |
| for (i=0; i<N; i++){ | | |
| OUI | 24 | bslbf |
| selector_length | 8 | uimsbf |
| for (i=0; i<N; i++){ | | |
| selector_byte | 8 | uimsbf |
| } | | |
| } | | |
| for (i=0; i<N; i++){ | | |
| private_data_byte | 8 | uimsbf |
| } | | |
| } | | |

