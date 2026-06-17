## Table 8 — MISO group function
_§5.2.8.2.2, PDF p.23_

| Syntax | Number of bits | Format |
|---|---|---|
| tx_MISO_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| MISO_group | 1 | bflbf |
| reserved_for_future_use | 7 | bflbf |
| } | | |
| } | | |

