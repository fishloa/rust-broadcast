## Table 16 — Default authority descriptor
_§6.3.3, PDF pp. 29-29_

| Syntax | No. of bits | Identifier |
|---|---|---|
| default authority descriptor() { | | |
| descriptor tag | 8 | uimsbf |
| descriptor length | 8 | uimsbf |
| for (i=0; i < descriptor length; i++) { | | |
| default authority byte | 8 | uimsbf |
| } | | |
| } | | |

