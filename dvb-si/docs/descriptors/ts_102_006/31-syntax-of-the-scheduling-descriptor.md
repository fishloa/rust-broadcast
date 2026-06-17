## Table 31 — Syntax of the scheduling descriptor
_§9.5.2.8, PDF pp. 30-30_

| Syntax | No. of | Identifier | Default value |
|---|---|---|---|
| | bits | | |
| scheduling_descriptor() { | | | |
| descriptor_tag | 8 | uimsbf | 0x01 |
| descriptor_length | 8 | uimsbf | |
| start_date_time | 40 | uimsbf | |
| end_date_time | 40 | uimsbf | |
| final_availability | 1 | bslbf | |
| periodicity_flag | 1 | bslbf | 0: not periodic. 1: periodic. |
| period_unit | 2 | bslbf | see Table 32 |
| duration_unit | 2 | bslbf | see Table 32 |
| estimated_cycle_time_unit | 2 | bslbf | see Table 32 |
| period | 8 | uimsbf | |
| duration | 8 | uimsbf | |
| estimated_cycle_time | 8 | uimsbf | |
| for (i=0; i<N; i++) { | | | |
| private_data_byte | 8 | uimsbf | |
| } | | | |
| } | | | |

