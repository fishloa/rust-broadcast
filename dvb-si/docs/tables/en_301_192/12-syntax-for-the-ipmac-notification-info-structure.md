## Table 12 — Syntax for the IP/MAC_notification_info structure
_§8.3.2, PDF pp. 25-27_

| Syntax | No. of bits | Identifier |
|---|---|---|
| IP/MAC_notification_info () { |  |  |
| platform_id_data_length | 8 | uimsbf |
| for (i=0; i<N; i++){ |  |  |
| platform_id | 24 | uimsbf |
| action_type | 8 | uimsbf |
| reserved | 2 | bslbf |
| INT_versioning_flag | 1 | bslbf |
| INT_version | 5 | uimsbf |
| } |  |  |
| for (i=0; i<N; i++){ |  |  |
| private_data_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

