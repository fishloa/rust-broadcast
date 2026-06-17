## Table 7 — ACE-PAPR function
_§5.2.8.2.1, PDF p.23_

| Syntax | Number of bits | Format |
|---|---|---|
| tx_ACE_PAPR_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| ACE_gain | 5 | uimsbf |
| ACE_maximal_extension | 3 | uimsbf |
| ACE_clipping_threshold | 7 | uimsbf |
| reserved_for_future_use | 1 | bflbf |
| } | | |
| } | | |

