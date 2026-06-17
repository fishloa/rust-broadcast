## Table 102 — Index structure for Schedule index by title
_§9.5.2.1, PDF pp. 89-89_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleCridIndex() { | | | |
| Overlapping subindices | 1 | '0' | no overlapped indexing. |
| Single layer sub index | 1 | '0' | single layer only. |
| Reserved | 6 | '111111' | |
| fragment locator format | 8 | 0x01 | remote fragment_locators. |
| for (i=0; i<num sub indices; i++) { | | | |
| high field value | 16 | * | ref. to title string. |
| Schedule sub index container | 16 | + | the ID of the container carrying the ScheduleTitleSubIndex structure. |
| Schedule sub index identifier | 8 | + | the structure_id of the ScheduleTitleSubIndex structure. |
| } | | | |
| } | | | |

