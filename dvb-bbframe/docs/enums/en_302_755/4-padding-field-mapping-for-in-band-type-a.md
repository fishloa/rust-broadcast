## Table 4 — Padding field mapping for in-band type A
_§5.2.3.1, PDF p. 34_

| Field | Size |
|---|---|
| PADDING_TYPE ('00') | 2 bits |
| PLP_L1_CHANGE_COUNTER | 8 bits |
| RESERVED_1 | 8 bits |
| For j=0..PI-1 { |  |
| SUB_SLICE_INTERVAL | 22 bits |
| START_RF_IDX | 3 bits |
| CURRENT_PLP_START | 22 bits |
| RESERVED_2 | 8 bits |
| } |  |
| CURRENT_PLP_NUM_BLOCKS | 10 bits |
| NUM_OTHER_PLP_IN_BAND | 8 bits |
| For i=0..NUM_OTHER_PLP_IN_BAND-1 { |  |
| PLP_ID | 8 bits |
| PLP_START | 22 bits |
| PLP_NUM_BLOCKS | 10 bits |
| RESERVED_3 | 8 bits |
| } |  |
| For j=0..PI-1 { |  |
| TYPE_2_START | 22 bits |
| } |  |

