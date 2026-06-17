## Table 52 — Syntax of info_descriptor
_§10.2.4, PDF pp. 65-65_

| info_descriptor(){ | No. of bytes | Value |
|---|---|---|
| descriptor_tag | 1 | 0x03 |
| descriptor_length | 1 |  |
| ISO_639_language_code | 3 |  |
| for (i=0; i<N;i++) { |  |  |
| text_char | 1 | Description of the module or group |
| } |  |  |
| } |  |  |

