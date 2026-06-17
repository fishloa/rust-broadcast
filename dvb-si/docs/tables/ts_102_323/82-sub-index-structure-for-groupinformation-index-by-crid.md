## Table 82 — Sub index structure for GroupInformation index by CRID
_§9.5.1.4.4, PDF pp. 79-79_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| GroupInfoCridSubIndex() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | Only one index layer so this is the leaf field. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each CRID indexed. |
| field value | 16 | * | ref. to GroupInformation CRID string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying GroupInfo fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

