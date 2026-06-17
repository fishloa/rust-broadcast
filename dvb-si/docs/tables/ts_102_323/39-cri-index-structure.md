## Table 39 — Cri_index structure
_§7.3.3.2, PDF pp. 49-49_

| Syntax | No. of bits | Identifier |
|---|---|---|
| cri index() { | | |
| overlapping subindices | 1 | bslbf |
| reserved other use | 1 | bslbf |
| reserved | 6 | bslbf |
| result locator format | 8 | uimsbf |
| for (i=0; i<prepend index count; i++) { | | |
| if (overlapping subindices == 1) { | | |
| low key value CRID | 16 | uimsbf |
| } | | |
| high key value CRID | 16 | uimsbf |
| prepend index container | 16 | uimsbf |
| prepend index identifier | 8 | uimsbf |
| } | | |
| } | | |

