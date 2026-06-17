## Table 32 — Syntax of selector bytes for interaction transport
_§5.3.6.2, PDF pp. 46-46_

| Syntax | Bits | Identifier |
|---|---|---|
| for( i=0; i<N; i++){ |  |  |
| URL_base_length | 8 | uimsbf |
| for( j=0; j<N; j++){ |  |  |
| URL_base_byte | 8 | uimsbf |
| } |  |  |
| URL_extension_count | 8 | uimsbf |
| for( j=0; j<URL_extension_count; j++){ |  |  |
| URL_extension_length | 8 | uimsbf |
| for(k=0; k<URL_length; k++){ |  |  |
| URL_extension_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |

