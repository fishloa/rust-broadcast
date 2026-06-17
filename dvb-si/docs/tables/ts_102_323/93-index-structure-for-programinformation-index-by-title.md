## Table 93 — Index structure for ProgramInformation index by title
_§9.5.1.7.3, PDF pp. 84-84_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridIndex() { | | | |
| Overlapping subindices | 1 | '0' | no overlapped indexing. |
| Single layer sub index | 1 | '0' | single layer only. |
| Reserved | 6 | '111111' | |
| fragment locator format | 8 | 0x01 | remote fragment_locators. |
| for (i=0; i<num sub indices; i++) { | | | |
| high field value | 16 | * | ref. to title string. |
| ProgramInfo sub index container | 16 | + | the ID of the container carrying the ProgramInfoTitleSubIndex structure. |
| ProgramInfo sub index identifier | 8 | + | the structure_id of the ProgramInfoTitleSubIndex structure. |
| } | | | |
| } | | | |

