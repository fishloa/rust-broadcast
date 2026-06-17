## Table 1 — Syntax for PES_data_packet structure
_§6.1, PDF pp. 15-15_

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| PES_data_packet () { |  |  |
| data_identifier | 8 | uimsbf |
| sub_stream_id | 8 | uimsbf |
| PTS_extension_flag | 1 | bslbf |
| output_data_rate_flag | 1 | bslbf |
| reserved | 2 | bslbf |
| PES_data_packet_header_length | 4 | uimsbf |
| if (PTS_extension_flag=="1") { |  |  |
| reserved | 7 | bslbf |
| PTS_extension | 9 | bslbf |
| } |  |  |
| if (output_data_rate_flag=="1") { |  |  |
| reserved | 4 | bslbf |
| output_data_rate | 28 | uimsbf |
| } |  |  |
| for (i=0;i<N;i++) { |  |  |
| PES_data_private_data_byte | 8 | bslbf |
| } |  |  |
| for (i=0;i<N;i++) { |  |  |
| PES_data_byte | 8 | bslbf |
| } |  |  |
| } |  |  |

> **Spec note:** The PDF Table 1 title says "Syntax for PES_data_packet structure"
> and the syntax uses `PES_data_packet ()`. An earlier PDF version used
> `PES_dataPacket` (camel-case) — v1.7.1 uses underscores throughout.

