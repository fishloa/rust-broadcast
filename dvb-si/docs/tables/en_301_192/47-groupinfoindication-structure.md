## Table 47 — GroupInfoIndication structure
_§10.1.3, PDF pp. 63-63_

| Syntax | Num. of bytes |
|---|---|
| GroupInfoIndication() { |  |
| NumberOfGroups | 2 |
| for(i=0;i< numberOfGroups;i++) { |  |
| GroupId | 4 |
| GroupSize | 4 |
| GroupCompatibility() |  |
| GroupInfoLength | 2 |
| for(i=0;i<N;I++) { |  |
| groupInfoByte | 1 |
| } |  |
| } |  |
| PrivateDataLength | 2 |
| for(i=0;i< privateDataLength;i++) { |  |
| privateDataByte | 1 |
| } |  |
| } |  |

