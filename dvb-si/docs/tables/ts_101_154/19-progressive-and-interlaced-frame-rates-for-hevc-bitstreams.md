## Table 19 — Progressive and Interlaced Frame Rates for HEVC Bitstreams
_§5.14.1.7, PDF pp. 127-127_

| Output Frame Rate | Interlaced or Progressive | elemental_duration_in_tc_minus1 [temporal_id_max (note 3)] | vui_time_scale | vui_num_units_in_tick | Allowed pic_struct |
|---|---|---|---|---|---|
| 24 000/1 001 | P | 0 | 24 000 | 1 001 | 0,7,8 |
| 24 | P | 0 | 24 | 1 | 0,7,8 |
| 25 | P | 0 | 25 | 1 | 0,7,8 |
| 25 | I (encoded as frames) | 0 | 50 | 1 | 3,4,5,6 |
| 25 | I (encoded as fields) | 0 | 50 | 1 | 9,10,11,12 |
| 30 000/1 001 | P | 0 | 30 000 | 1 001 | 0,7,8 |
| 30 000/1 001 | I (encoded as frames) | 0 | 60 000 | 1 001 | 3,4,5,6 |
| 30 000/1 001 | I (encoded as fields) | 0 | 60 000 | 1 001 | 9,10,11,12 |
| 30 | P | 0 | 30 | 1 | 0,7,8 |
| 50 | P | 0 | 50 | 1 | 0,7,8 |
| 60 000/1 001 | P | 0 | 60 000 | 1 001 | 0,7,8 |
| 60 | P | 0 | 60 | 1 | 0,7,8 |

