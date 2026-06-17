## Table 90 — Sub index structure for ProgramInformation index by CRID
_§9.5.1.6.4, PDF pp. 83-83_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ProgramInfoCridSubIndex() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | Only one index layer so this is the leaf field. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each CRID indexed. |
| field value | 16 | * | ref. to ProgramInformation CRID string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying ProgramInfo fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

