## Table 97 — Index structure for Schedule index by time and DVB service
_§9.5.1.8.3, PDF pp. 86-86_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleTimeServiceIndex() { | | | |
| Overlapping subindices | 1 | '0' | no overlapped indexing. |
| Single layer sub index | 1 | '0' | single layer only. |
| Reserved | 6 | '111111' | |
| fragment locator format | 8 | 0x01 | remote fragment_locators. |
| for (i=0; i<num sub indices; i++) { | | | |
| high field value1 | 16 | * | ref. to date string. |
| high field value2 | 16 | * | ref. to serviceIdRef string. |
| Schedule sub index container | 16 | + | the ID of the container carrying the ScheduleCridSubIndex structure. |
| Schedule sub index identifier | 8 | + | the structure_id of the ScheduleCridSubIndex structure. |
| } | | | |
| } | | | |

