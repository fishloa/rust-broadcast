## Table 86 — Sub index structure for GroupInformation index by title
_§9.5.1.5.4, PDF pp. 81-81_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| GroupInfoTitleSubIndex() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | Only one index layer so this is the leaf field. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each title indexed. |
| field value | 16 | * | ref. to GroupInformation title string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying GroupInformation fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

