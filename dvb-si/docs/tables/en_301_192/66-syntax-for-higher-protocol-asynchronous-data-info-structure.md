## Table 66 — Syntax for higher_protocol_asynchronous_data_info structure
_§12.2.1, PDF pp. 74-74_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| higher_protocol_asynchronous_data_info () { |  |  |
| higher_protocol_id | 4 | uimsbf |
| reserved | 4 | bslbf |
| for (i=0; i<N;i++){ |  |  |
| private_data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |

