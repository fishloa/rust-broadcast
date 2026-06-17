## Table 9 — TR-PAPR function
_§5.2.8.2.3, PDF p.24_

| Syntax | Number of bits | Format |
|---|---|---|
| tx_TR_PAPR_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| reserved_for_future_use1 | 4 | bflbf |
| TR_clipping_threshold | 12 | uimsbf |
| reserved_for_future_use2 | 14 | |
| number_of_iterarions | 10 | bflbf |
| } | | |
| } | | |

> **Spec note:** The PDF prints `number_of_iterarions` (with the typo "iterarions") — reproduced verbatim.

