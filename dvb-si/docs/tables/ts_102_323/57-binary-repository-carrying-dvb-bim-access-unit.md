## Table 57 — binary repository carrying DVB BiM access unit
_§9.4.2.3, PDF pp. 62-62_

| Syntax | No. of bits | Identifier |
|---|---|---|
| binary repository() { | | |
| DVBBiMAccessUnit { | | |
| NumberOfFUU | 8+ | vluimsbf8 |
| for(i=0; i< NumberOfFUU; i++) { | | |
| FUULength | 8+ | vluimsbf8 |
| DVBContextPath | 16 | uimsbf |
| FragmentUpdatePayload(startType) | | |
| } | | |
| } | | |
| for (i=0; i<N; i++) { | | |
| private byte | 8 | uimsbf |
| } | | |
| } | | |

