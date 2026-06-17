## Table 101 — Index List entry for Schedule index by Title
_§9.5.1.9.2, PDF pp. 88-88_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleTitleIndexListEntry() { | | | |
| index descriptor length | 8 | 0x0C | |
| fragment type | 16 | 0x0002 | indicates Schedule fragments. |
| num fields | 8 | 0x01 | single key index. |
| field identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for field. |
| field xpath ptr | 16 | * | ref. to string "tva:ScheduleEvent/tva:InstanceDescription/tva:Title.text()". |
| field encoding | 16 | 0x0000 | indicates no encoding for field entries in ScheduleTitleIndex or ScheduleTitleSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the ScheduleTitleIndex structure. |
| index identifier | 8 | + | the structure_id of the ScheduleTitleIndex structure. |
| } | | | |

