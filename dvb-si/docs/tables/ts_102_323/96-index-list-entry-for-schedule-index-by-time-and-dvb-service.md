## Table 96 — Index List entry for Schedule index by time and DVB service
_§9.5.1.8.2, PDF pp. 86-86_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleTimeServiceIndexListEntry() { | | | |
| index descriptor length | 8 | 0x12 | |
| fragment type | 16 | 0x0005 | indicates Schedule fragment. |
| num fields | 8 | 0x02 | two key index. |
| field1 identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for first key field. |
| field1 xpath ptr | 16 | * | ref. to string "@tva:end". |
| field1 encoding | 16 | 0x0000 | indicates no encoding for first key field entries in ScheduleTimeServiceIndex or ScheduleTimeServiceSubIndex structures. |
| field2 identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for second key field. |
| field2 xpath ptr | 16 | * | ref. to string "@tva:serviceIdRef". |
| field2 encoding | 16 | 0x0000 | indicates no encoding for second key field entries in ScheduleTimeServiceIndex or ScheduleTimeServiceSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the ScheduleTimeServiceIndex structure. |
| index identifier | 8 | + | the structure_id of the ScheduleTimeServiceIndex structure. |
| } | | | |

