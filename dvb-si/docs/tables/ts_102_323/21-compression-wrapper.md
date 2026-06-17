## Table 21 — Compression_wrapper
_§7.3.2.1, PDF pp. 37-37_

| Syntax | No. of bits | Identifier |
|---|---|---|
| compression wrapper() { | | |
| compression method | 8 | uimsbf |
| if (compression method == 0x00) { | | |
| container() | | |
| } else if (compression method == 0x01) { | | |
| original size | 24 | uimsbf |
| compression structure() | N x 8 | |
| } | | |

