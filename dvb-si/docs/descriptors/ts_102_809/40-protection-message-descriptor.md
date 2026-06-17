## Table 40 — Protection message descriptor
_§9.3.3, PDF pp. 65-65_

| Syntax | Number of bits | Identifier |
|---|---|---|
| protection_message_descriptor(){ |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| reserved | 4 | bslbf |
| component_count | 4 | uimsbf |
| for (i=0;i<N;i++){ |  |  |
| component_tag | 8 | uimsbf |
| } |  |  |
| } |  |  |

