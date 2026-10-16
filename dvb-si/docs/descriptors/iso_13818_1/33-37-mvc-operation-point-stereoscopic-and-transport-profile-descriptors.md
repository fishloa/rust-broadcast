## Table 2-103 — MVC operation point descriptor (tag 0x33)
_Rec. ITU-T H.222.0 (08/2023) §2.6.82–2.6.83, Table 2-103; printed p.118 (PDF p.140). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| MVC_operation_point_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;profile_idc | 8 | uimsbf |
| &nbsp;&nbsp;constraint_set0_flag … constraint_set5_flag (six 1-bit fields) | 1 each | bslbf |
| &nbsp;&nbsp;AVC_compatible_flags | 2 | bslbf |
| &nbsp;&nbsp;level_count | 8 | uimsbf |
| &nbsp;&nbsp;for (i = 0; i < level_count; i++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;level_idc | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;operation_points_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;for (j = 0; j < operation_points_count; j++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;applicable_temporal_id | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;num_target_output_views | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ES_count | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;for (k = 0; k < ES_count; k++) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;reserved | 2 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;ES_reference | 6 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;} |  |  |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

`ES_count` value 0xFF is reserved. (The six flags are printed as six separate rows in the table.)

## Table 2-104 — MPEG2_stereoscopic_video_format_descriptor (tag 0x34)
_§2.6.84–2.6.85, Table 2-104; printed p.119 (PDF p.141). Verified against the rendered page._

| Syntax | No. of bits | Format |
|---|---|---|
| MPEG2_stereoscopic_video_format_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;stereo_video_arrangement_type_present | 1 | bslbf |
| &nbsp;&nbsp;if (stereo_video_arrangement_type_present) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;arrangement_type | 7 | bslbf |
| &nbsp;&nbsp;} else { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

## Table 2-105 — Stereoscopic_program_info_descriptor (tag 0x35)
_§2.6.86–2.6.87, Tables 2-105/2-106; printed p.119–120 (PDF p.141–142). Verified against the rendered pages._

| Syntax | No. of bits | Format |
|---|---|---|
| Stereoscopic_program_info_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 5 | bslbf |
| &nbsp;&nbsp;stereoscopic_service_type | 3 | bslbf |
| } |  |  |

`descriptor_length` shall be 0x01. Table 2-106 `stereoscopic_service_type`: `000` unspecified; `001` 2D-only (monoscopic); `010` frame-compatible stereoscopic 3D; `011` service-compatible stereoscopic 3D; `100`..`111` reserved.

## Table 2-107 — Stereoscopic_video_info_descriptor (tag 0x36)
_§2.6.88–2.6.89, Tables 2-107/2-108; printed p.120–121 (PDF p.142–143). Verified against the rendered page._

| Syntax | No. of bits | Format |
|---|---|---|
| Stereoscopic_video_info_descriptor() { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;base_video_flag | 1 | bslbf |
| &nbsp;&nbsp;if (base_video_flag) { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;leftview_flag | 1 | bslbf |
| &nbsp;&nbsp;} else { |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;usable_as_2D | 1 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;horizontal_upsampling_factor | 4 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;vertical_upsampling_factor | 4 | bslbf |
| &nbsp;&nbsp;} |  |  |
| } |  |  |

Table 2-108 upsampling factor: `0000` forbidden; `0001` unspecified; `0010` same coded resolution as base view; `0011` 3/4; `0100` 2/3; `0101` 1/2; `0110`..`1000` reserved; `1001`..`1111` user_private.

## Table 2-111 — Transport_profile_descriptor (tag 0x37)
_§2.6.93–2.6.94, Tables 2-111/2-112; printed p.125 (PDF p.147). Verified against the rendered page._

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| Transport_profile_descriptor { |  |  |
| &nbsp;&nbsp;descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;transport_profile | 8 | uimsbf |
| &nbsp;&nbsp;for (i=0; i<N; i++) { private_data | 8 | bslbf |
| } |  |  |

Table 2-112 `transport_profile`: 0x00 unspecified; 0x01 Complete profile; 0x02 Adaptive profile; 0x03..0x0E reserved; 0x0F..0xFF user_private.
