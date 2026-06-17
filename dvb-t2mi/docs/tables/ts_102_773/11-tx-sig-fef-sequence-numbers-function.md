## Table 11 — TX-SIG FEF Sequence Numbers function
_§5.2.8.2.5, PDF p.25_

| Syntax | Number of bits | Format |
|---|---|---|
| tx_TX_SIG_SEQ_NUM_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| reserved_for_future_use1 | 5 | bflbf |
| TX_SIG_FEF_SEQ_NUM_1 | 3 | uimsbf |
| reserved_for_future_use2 | 5 | bflbf |
| TX_SIG_FEF_SEQ_NUM_2 | 3 | uimsbf |
| reserved_for_future_use3 | 24 | bflbf |
| } | | |
| } | | |

