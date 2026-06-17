## Table 21b — Progressive Frame Rates for HEVC HFR UHDTV Bitstreams
_§5.14.5.5.1, PDF pp. 145-145_

| Output Frame Rate (fps) — HEVC UHDTV IRD | Output Frame Rate (fps) — HEVC HDR HFR UHDTV IRD | Stream Type: 0x24 (HEVC bitstream and HEVC temporal video sub-bitstream) elemental_duration_in_tc_minus1 [temporal_id_max](0x24) | Stream Type: 0x25 (HEVC temporal video subset) elemental_duration_in_tc_minus1 [temporal_id_max](0x25) | vui_time_scale | vui_num_units_in_tick | Allowed pic_struct |
|---|---|---|---|---|---|---|
| Not applicable | 100 | 0 | Not applicable | 100 | 1 | 0,7,8 |
| 50 | 100 | 1 | 0 | 100 | 1 | 0,7,8 |
| Not applicable | 120 000/1 001 | 0 | Not applicable | 120 000 | 1 001 | 0,7,8 |
| 60 000/1 001 | 120 000/1 001 | 1 | 0 | 120 000 | 1 001 | 0,7,8 |
| Not applicable | 120 | 0 | Not applicable | 120 | 1 | 0,7,8 |
| 60 | 120 | 1 | 0 | 120 | 1 | 0,7,8 |

NOTE: If the HEVC temporal video subset is either not applicable, not present or not decoded, the HEVC Output Frame Rate is calculated using vui_time_scale, vui_num_units_in_tick and elemental_duration_in_tc_minus1[temporal_id_max](0x24).

