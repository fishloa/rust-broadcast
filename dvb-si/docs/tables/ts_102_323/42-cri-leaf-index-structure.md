## Table 42 — Cri_leaf_index structure
_§7.3.3.4, PDF pp. 51-51_

| Syntax | No. of bits | Identifier |
|---|---|---|
| cri leaf index() { | | |
| leaf flag | 1 | bslbf |
| reserved | 7 | uimsbf |
| for (j=0; j<reference count; j++) { | | |
| variable CRID data | 16 | uimsbf |
| result locator() | variable | |
| } | | |
| } | | |

