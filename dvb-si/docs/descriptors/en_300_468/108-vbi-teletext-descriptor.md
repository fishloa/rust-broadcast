## Table 108 — VBI teletext descriptor
_§6.3, PDF pp. 111-111_

| Syntax | Number of bits | Identifier |
|---|---|---|
| VBI_teletext_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| ISO_639_language_code | 24 | bslbf |
| teletext_type | 5 | uimsbf |
| teletext_magazine_number | 3 | uimsbf |
| teletext_page_number | 8 | uimsbf |
| } |
| } |

