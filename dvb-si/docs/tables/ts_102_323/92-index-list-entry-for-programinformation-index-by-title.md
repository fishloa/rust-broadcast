## Table 92 — Index List entry for ProgramInformation index by title
_§9.5.1.7.2, PDF pp. 84-84_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridIndexListEntry() { | | | |
| index descriptor length | 8 | 0x0C | |
| fragment type | 16 | 0x0001 | indicates ProgramInformation fragments. |
| num fields | 8 | 0x01 | single key index. |
| field identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for field. |
| field xpath ptr | 16 | * | ref. to string "tva:BasicDescription/tva:Title.text()". |
| field encoding | 16 | 0x0000 | indicates no encoding for field entries in ProgramInfoTitleIndex or ProgramInfoTitleSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the ProgramInfoTitleIndex structure. |
| index identifier | 8 | + | the structure_id of the ProgramInfoTitleIndex structure. |
| } | | | |

