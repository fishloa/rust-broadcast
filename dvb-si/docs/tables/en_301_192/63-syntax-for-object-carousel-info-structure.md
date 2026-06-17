## Table 63 — Syntax for object_carousel_info structure
_§11.3.2, PDF pp. 72-72_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| object_carousel_info () { |  |  |
| carousel_type_id | 2 | bslbf |
| reserved | 6 | bslbf |
| transaction_id | 32 | uimsbf |
| time_out_value_DSI | 32 | uimsbf |
| time_out_value_DII | 32 | uimsbf |
| reserved | 2 | bslbf |
| leak_rate | 22 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| ISO_639_language_code | 24 | bslbf |
| object_name_length | 8 | uimsbf |
| for (i=0;i<N;i++) { |  |  |
| object_name_char | 8 | uimsbf |
| } |  |  |
| } |  |  |
| } |  |  |

