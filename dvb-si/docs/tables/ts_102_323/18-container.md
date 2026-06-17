## Table 18 — Container
_§7.3.1.3, PDF pp. 35-35_

| Syntax | No. of bits | Identifier |
|---|---|---|
| container() { | | |
| container header { | | |
| num cri structures | 8 | uimsbf |
| for(j=0; j<num cri structures; j++) { | | |
| cri structure type | 8 | uimsbf |
| cri structure id | 8 | uimsbf |
| cri structure ptr | 24 | uimsbf |
| cri structure length | 24 | uimsbf |
| } | | |
| } | | |
| for (j=0; j<num cri structures; j++) { | | |
| cri structure() | | |
| } | | |
| } | | |

