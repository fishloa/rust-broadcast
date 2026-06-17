## Table 80 — Index list entry for GroupInformation index by CRID
_§9.5.1.4.2, PDF pp. 78-78_

| Syntax | No. of bits | Value | Comments |
|---|---|---|---|
| GroupInfoCridIndexListEntry() { | | | |
| index descriptor length | 8 | 0x0C | |
| fragment type | 16 | 0x0002 | indicates GroupInformation fragment. |
| num fields | 8 | 0x01 | single key index. |
| field identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for field. |
| field xpath ptr | 16 | * | ref. to string "@tva:groupId". |
| field encoding | 16 | 0x0000 | indicates no encoding for field entries in GroupInfoCridIndex or GroupInfoCridSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the GroupInfoCridIndex structure. |
| index identifier | 8 | + | the structure_id of the GroupInfoCridIndex structure. |
| } | | | |

