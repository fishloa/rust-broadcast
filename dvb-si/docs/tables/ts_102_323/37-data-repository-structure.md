## Table 37 — Data repository structure
_§7.3.3.1, PDF pp. 48-48_

| Syntax | No. of bits | Identifier |
|---|---|---|
| data repository() { | | |
| string encoding | 8 | uimsbf |
| for (i=0; i<item count; i++) { | | |
| if (string encoding < 0x03) { | | |
| for (j=0; j<stringlength; j++) { | | |
| string character | 8 | uimsbf |
| } | | |
| if(string encoding == 0x00){ | | |
| 0x00 | 8 | uimsbf |
| } else if(string encoding == 0x01){ | | |
| 0x00 | 8 | uimsbf |
| } else if(string encoding == 0x02){ | | |
| 0x0000 | | |
| } | | |
| else { /* string encoding >= 0x03*/ | | |
| for (j=0; j<stringlength; j++) { | | |
| private byte | 8 | uimsbf |
| } | | |
| } | | |
| } | | |
| } | | |

