## Table 88 — Index List entry for ProgramInformation index by CRID
_§9.5.1.6.2, PDF pp. 82-82_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridIndexListEntry() { | | | |
| index descriptor length | 8 | 0x0C | |
| fragment type | 16 | 0x0001 | indicates ProgramInformation fragment. |
| num fields | 8 | 0x01 | single key index. |
| field identifier | 16 | 0xFFFF | indicates use of W3C Xpath expression for field. |
| field xpath ptr | 16 | * | ref. to string "@tva:programId". |
| field encoding | 16 | 0x0000 | indicates no encoding for field entries in ProgramInfoCridIndex or ProgramInfoCridSubIndex structures. |
| container id | 16 | + | the ID of the container carrying the ProgramInfoCridIndex structure. |
| index identifier | 8 | + | the structure_id of the ProgramInfoCridIndex structure. |
| } | | | |

