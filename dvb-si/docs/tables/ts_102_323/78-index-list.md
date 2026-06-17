## Table 78 — Index list
_§9.5.1.3, PDF pp. 77-77_

| Syntax | No. of bits | Identifier | Value | Comment |
|---|---|---|---|---|
| index list() { | | | | |
| for (i=0; i<N; i++) { | | | | |
| Either GroupInfoCridIndexListEntry() | | | | entries for profiled indices |
| OR GroupInfoTitleIndexListEntry() | | | | |
| OR ProgramInfoCridIndexListEntry() | | | | |
| OR ProgramInfoTitleIndexListEntry() | | | | |
| OR ScheduleTimeServiceIndexListEntry() | | | | |
| OR ScheduleTitleIndexListEntry() | | | | |
| OR { | | | | generic index entry |
| index descriptor length | 8 | uimsbf | + (see note 1) | |
| fragment type | 16 | uimsbf | + | |
| if (fragment type == 0xFFFF) { | | | | |
| fragment xpath ptr | 16 | | | if fragment_type==0xFFFF this is ref. (see note 2) to XPath string |
| } | | | | |
| num fields | 8 | uimsbf | + | |
| for (i=0; i<num fields; i++) { | | | | |
| field identifier | 16 | uimsbf | + | 0xFFFF indicates use of W3C Xpath expression for field. |
| if (field identifier == 0xFFFF) { | | | | |
| field xpath ptr | 16 | uimsbf | * (see note 3) | if field_identifier==0xFFFF this is ref. to XPath string |
| } | | | | |
| field encoding | 16 | uimsbf | + | |
| } | | | | |
| container id | 16 | uimsbf | + | |
| index identifier | 8 | uimsbf | + | |
| } | | | | |
| } | | | | |
| } | | | | |
| NOTE 1: "+" indicates that the value is assigned (e.g. an identifier value). NOTE 2: References to strings are all offsets from the start of the string_repository carried in the same container. NOTE 3: "*" indicates that the value is calculated (e.g. a length or an offset value). | | | | |

