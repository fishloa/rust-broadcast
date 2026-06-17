## Table 103 — Sub index structure for Schedule index by title
_§9.5.2.1, PDF pp. 89-89_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleCridSubIndex() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | Only one index layer so this is the leaf field. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each title indexed. |
| field value | 16 | * | ref. to Schedule title string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying Schedule fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

