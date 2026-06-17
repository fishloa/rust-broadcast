## Table 98 — First layer sub index structure for Schedule index by time and DVB service
_§9.5.1.8.4, PDF pp. 87-87_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleCridSubIndex1() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '0' | this is not the last sub index layer. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| child sub index ref | 8 | + | the structure_id of the associate 2nd layer sub index structure. |
| for (j=0; j<num entries;j++) { | | | repeat for each DVB service indexed in this container. |
| field value | 16 | * | ref. to end time string. |
| range end offset | 16 | + | container carrying Schedule fragment. |
| } | | | |
| } | | | |

