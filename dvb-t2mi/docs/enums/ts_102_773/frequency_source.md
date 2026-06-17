## Table 12b — Frequency function
_§5.2.8.2.7, PDF p.26_

| Syntax | Number of bits | Format |
|---|---|---|
| frequency_function() { | | |
| function_tag | 8 | uimsbf |
| function_length | 8 | uimsbf |
| function_body() { | | |
| rf_idx | 3 | uimsbf |
| frequency | 32 | uimsbf |
| reserved | 5 | bflbf |
| } | | |
| } | | |

