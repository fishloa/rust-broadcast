## Table 10 — L1-ACE-PAPR function
_§5.2.8.2.4, PDF p.25_

| Syntax | Number of bits | Format |
|---|---|---|
| tx_L1_ACE_PAPR_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| L1_ACE_max_correction | 16 | uimsbf |
| reserved_for_future_use | 16 | bflbf |
| } | | |
| } | | |

