## Table 52 — DSNG descriptor
_§6.2.15, PDF pp. 78-78_

| Syntax | Number of bits | Identifier |
|---|---|---|
| DSNG_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| byte | 8 | uimsbf |
| } |
| } |

