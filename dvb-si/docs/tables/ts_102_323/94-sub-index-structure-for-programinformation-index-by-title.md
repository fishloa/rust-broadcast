## Table 94 — Sub index structure for ProgramInformation index by title
_§9.5.1.7.4, PDF pp. 85-85_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridSubIndex() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | Only one index layer so this is the leaf field. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each title indexed. |
| field value | 16 | * | ref. to ProgramInformation title string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying ProgramInformation fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

