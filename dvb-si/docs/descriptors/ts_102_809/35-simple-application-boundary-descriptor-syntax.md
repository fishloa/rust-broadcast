## Table 35 — Simple application boundary descriptor syntax
_§5.3.8, PDF pp. 48-48_

|  | No.of bits | Identifier | Value |
|---|---|---|---|
| simple_application_boundary_descriptor { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x17 |
| descriptor_length | 8 | uimsbf |  |
| boundary_extension_count | 8 | uimsbf |  |
| for( j=0; j<boundary_extension_count; j++){ |  |  |  |
| boundary_extension_length | 8 | uimsbf |  |
| for(k=0; k<boundary_extension_length; k++){ |  |  |  |
| boundary_extension_byte | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

