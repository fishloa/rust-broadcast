## Table 38 — String encoding
_§7.3.3.1, PDF pp. 48-48_

| Value | Description | Termination Value |
|---|---|---|
| 0x00 | 8 bit ASCII (ISO/IEC 646 [24]) | 0x00 |
| 0x01 | UTF-8 | 0x00 |
| 0x02 | UTF-16 | 0x0000 |
| 0x03 to 0xE0 | reserved | undefined |
| 0xE1 to 0xFF | User Private | undefined |

