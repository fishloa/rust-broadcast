## Table 6 — GroupInfoIndication structure
_§8.1.1, PDF pp. 16-16_

| Syntax | No. of | Remarks |
|---|---|---|
| | bytes | |
| GroupInfoIndication() { | | |
| NumberOfGroups | 2 | number of updates (maximum 150) |
| for (i=0; i<N; i++) { | | |
| GroupId | 4 | |
| GroupSize | 4 | |
| GroupCompatibility | variable | see Table 7 |
| GroupInfoLength | 2 | |
| for (i=0; i<N; i++) { | | |
| GroupInfoByte | 1 | |
| } | | |
| PrivateDataLength | 2 | see note |
| for (i=0; i<N; i++) { | | |
| PrivateDataByte | 1 | see note |
| } | | |
| } | | |
| } | | |

