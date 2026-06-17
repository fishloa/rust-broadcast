## Table 159 — URI linkage descriptor
_§6.4.16.1, PDF pp. 146-146_

| Syntax | Number of bits | Identifier |
|---|---|---|
| URI_linkage_descriptor() { |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| uri_linkage_type | 8 | uimsbf |
| uri_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |
| uri_char | 8 | bslbf |
| } |
| if ((uri_linkage_type == 0x00) |
| \|\| (uri_linkage_type == 0x01)) { |
| min_polling_interval | 16 | uimsbf |
| } |
| for (i=0;i<N;i++) { |
| private_data_byte | 8 | bslbf |
| } |
| } |

