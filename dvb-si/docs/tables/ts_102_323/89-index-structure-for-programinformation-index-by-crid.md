## Table 89 — Index structure for ProgramInformation index by CRID
_§9.5.1.6.3, PDF pp. 82-82_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridIndex() { | | | |
| Overlapping subindices | 1 | '0' | no overlapped indexing. |
| Single layer sub index | 1 | '0' | single layer only. |
| Reserved | 6 | '111111' | |
| fragment locator format | 8 | 0x01 | remote fragment_locators. |
| for (i=0; i<num sub indices; i++) { | | | |
| high field value | 16 | * | ref. to CRID string. |
| ProgramInfo sub index container | 16 | + | the ID of the container carrying the ProgramInfoCridSubIndex structure. |
| ProgramInfo sub index identifier | 8 | + | the structure_id of the ProgramInfoCridSubIndex structure. |
| } | | | |
| } | | | |

