## Table 38 — Syntax of the telephone_descriptor
_§8.4.5.16, PDF pp. 40-40_

| Name | No. of bits | Identifier | Remarks |
|---|---|---|---|
| telephone_descriptor() { |  |  |  |
| descriptor_tag | 8 | uimsbf | 0x57 |
| descriptor_length | 8 | uimsbf |  |
| reserved_future_use | 2 | bslbf |  |
| foreign_availability | 1 | bslbf |  |
| connection_type | 5 | uimsbf |  |
| reserved_future_use | 1 | uimsbf |  |
| country_prefix_length | 2 | uimsbf |  |
| international_area_code_char | 3 | uimsbf |  |
| operator_code_length | 2 | uimsbf |  |
| reserved_future_use | 1 | bslbf |  |
| national_area_code_length | 3 | uimsbf |  |
| core_number_length | 4 | uimsbf |  |
| for (i=0; i<N; i++){ |  |  |  |
| country_prefix_char | 8 | uimsbf |  |
| } |  |  |  |
| for (i=0; i<N; i++){ |  |  |  |
| international_area_code_char | 8 | uimsbf |  |
| } |  |  |  |
| for (i=0; i<N; i++){ |  |  |  |
| operator_code_char | 8 | uimsbf |  |
| } |  |  |  |
| for (i=0; i<N; i++){ |  |  |  |
| national_area_code_char | 8 | uimsbf |  |
| } |  |  |  |
| for (i=0; i<N; i++){ |  |  |  |
| core_number_char | 8 | uimsbf |  |
| } |  |  |  |
| } |  |  |  |

> **Spec note:** the 3-bit packed field is labelled `international_area_code_char` in the Table 38 syntax (reproduced verbatim from PDF p.40), but §8.4.5.17 semantics define it as `international_area_code_length` ("this 3-bit field specifies the number of 8-bit alphanumeric characters in the international area code"). The 8-bit field of the same name inside the loop is the actual area-code character. This duplicate-name labelling is an editorial inconsistency in EN 301 192 V1.7.1 (the same quirk appears in TS 102 006 Table 33).

