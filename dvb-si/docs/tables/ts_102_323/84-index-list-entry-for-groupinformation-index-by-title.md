## Table 84 — Index List entry for GroupInformation index by title
_§9.5.1.5.2, PDF pp. 80-80_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| GroupInfoCridIndexListEntry() { | | | |
| index descriptor length | 8 | 0x0C | |
| fragment type | 16 | 0x0002 | indicates GroupInformation fragments. |
| num fields | 8 | 0x01 | single key index. |
| field identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for field. |
| field xpath ptr | 16 | * | ref. to string "tva:BasicDescription/tva:Title.text()". |
| field encoding | 16 | 0x0000 | indicates no encoding for field entries in GroupInfoTitleIndex or GroupInfoTitleSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the GroupInfoTitleIndex structure. |
| index identifier | 8 | + | the structure_id of the GroupInfoTitleIndex structure. |
| } | | | |

