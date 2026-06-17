## Table 41 — Cri_prepend_index structure
_§7.3.3.4, PDF pp. 51-51_

| Syntax | No. of bits | Identifier |
|---|---|---|
| cri prepend index() { | | |
| leaf flag | 1 | bslbf |
| reserved | 7 | uimsbf |
| sub index ref | 8 | uimsbf |
| for (j=0; j<reference count; j++) { | | |
| prepend CRID data | 16 | uimsbf |
| range end offset | 16 | uimsbf |
| } | | |
| } | | |

