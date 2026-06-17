## Table 33 — Syntax of the telephone_descriptor
_§9.5.2.8, PDF pp. 31-31_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| telephone_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x57 |
| descriptor_length | 8 | uimsbf | |
| reserved_future_use | 2 | bslbf | |
| foreign_availability | 1 | bslbf | |
| connection_type | 5 | uimsbf | |
| reserved_future_use | 1 | bslbf | |
| country_prefix_length | 2 | uimsbf | |
| international_area_code_char | 3 | uimsbf | |
| operator_code_length | 2 | uimsbf | |
| reserved_future_use | 1 | bslbf | |
| national_area_code_length | 3 | uimsbf | |
| core_number_length | 4 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| country_prefix_char | 8 | uimsbf | |
| } | | | |
| for (i=0; i<N; i++) { | | | |
| international_area_code_char | 8 | uimsbf | |
| } | | | |
| for (i=0; i<N; i++) { | | | |
| operator_code_char | 8 | uimsbf | |
| } | | | |
| for (i=0; i<N; i++) { | | | |
| national_area_code_char | 8 | uimsbf | |
| } | | | |
| for (i=0; i<N; i++) { | | | |
| core_number_char | 8 | uimsbf | |
| } | | | |
| } | | | |

> **Spec note:** the 3-bit packed field is labelled `international_area_code_char` in the Table 33 syntax (reproduced verbatim), but §9.5.2.10 semantics define it as `international_area_code_length` ("this 3-bit field specifies the number of 8-bit alphanumeric characters in the international area code"). The 8-bit field of the same name inside the loop is the actual area-code character. This duplicate-name labelling is an editorial inconsistency in TS 102 006 V1.4.1.

