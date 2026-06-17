## Table 99 — Second layer sub index structure for Schedule index by time and DVB service
_§9.5.1.8.5, PDF pp. 87-87_

| Syntax | No. of bits | Value | Description |
|---|---|---|---|
| ScheduleCridSubIndex2() { | | | |
| { | | | multi_field_header. |
| leaf field | 1 | '1' | this is the last sub index layer. |
| multiple locators | 1 | '0' | fragment locators are in-line. |
| Reserved | 6 | '111111' | |
| } | | | |
| for (j=0; j<num entries;j++) { | | | repeat for each schedule fragment indexed in this container. |
| field value | 16 | * | ref. to Schedule service string. |
| { | | | inline fragment locator structure referencing remote fragments. |
| target container | 16 | + | container carrying Schedule fragment. |
| target fragment | 24 | + | unique fragment ID. |
| } | | | |
| } | | | |
| } | | | |

