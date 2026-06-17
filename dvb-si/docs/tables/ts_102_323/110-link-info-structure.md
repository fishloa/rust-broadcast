## Table 110 — Link info structure
_§10.4.3, PDF pp. 97-97_

| Syntax | No. of bits | Identifier |
|---|---|---|
| link info() { | | |
| link type | 4 | uimsbf |
| reserved future use | 2 | bslbf |
| how related classification scheme id | 6 | uimsbf |
| term id | 12 | uimsbf |
| group id | 4 | uimsbf |
| precedence | 4 | uimsbf |
| if (link type == 0x00 \|\| link type == 0x02) { | | |
| media uri length | 8 | uimsbf |
| for (k=0; k<media uri length; k++) { | | |
| media uri byte | 8 | uimsbf |
| } | | |
| } | | |
| if (link type == 0x01 \|\| link type == 0x02) { | | |
| dvb binary locator() | | |
| } | | |
| reserved future use | 2 | bslbf |
| number items | 6 | uimsbf |
| for (m=0; m<number items; m++) { | | |
| ISO 639-2 [23] language code | 24 | bslbf |
| promotional text length | 8 | uimsbf |
| for (n=0; n< promotional text length; n++) { | | |
| promotional text char | 8 | uimsbf |
| } | | |
| } | | |
| default icon flag | 1 | bslbf |
| icon id | 3 | uimsbf |
| descriptor loop length | 12 | uimsbf |
| for (p=0; p<descriptor loop length; p++) { | | |
| descriptor() | 8 | uimsbf |
| } | | |
| } | | |

