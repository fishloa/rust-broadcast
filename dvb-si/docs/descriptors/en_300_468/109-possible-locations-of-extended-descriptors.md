## Table 109 — Possible locations of extended descriptors
_§6.4.0, PDF pp. 112-112_

| image_icon_descriptor | 0x00 |  |  |  |  |  |  | - | - |
|---|---|---|---|---|---|---|---|
|  |  |  |  |  | ✓ |  | ✓ |
| cpcm_delivery_signalling_descriptor (see ETSI | 0x01 | - |  | - |  |  |  | - | - |  | - |
| TS 102 825 [27] and ETSI TR 102 825 [i.5]) |
|  |  |  |  |  |  |  |  |  | ✓ |
| CP_descriptor (see ETSI TS 102 825 [27] and ETSI | 0x02 | - |  | - | - |  | - | - |  |  | - |
| TR 102 825 [i.5]) |
|  |  | ✓ |  | ✓ | ✓ |  | ✓ |
| CP_identifier_descriptor (see ETSI TS 102 825 [27] | 0x03 |  |  |  |  |  |  | - | - |  | - |
| and ETSI TR 102 825 [i.5]) |
|  |  | ✓ |
| T2_delivery_system_descriptor | 0x04 |  |  | - | - |  | - | - | - |  | - |
|  |  | ✓ |
| SH_delivery_system_descriptor | 0x05 |  |  | - | - |  | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| supplementary_audio_descriptor | 0x06 | - |  | - | - |  | - | - |  |  | - |
|  |  | ✓ |
| network_change_notify_descriptor | 0x07 |  |  | - | - |  | - | - | - |  | - |
|  |  | ✓ |  | ✓ | ✓ |  | ✓ |
| message_descriptor | 0x08 |  |  |  |  |  |  | - | - |  | - |
|  |  | ✓ |  | ✓ | ✓ |
| target_region_descriptor | 0x09 |  |  |  |  |  | - | - | - |  | - |
|  |  | ✓ |  | ✓ |
| target_region_name_descriptor | 0x0A |  |  |  | - |  | - | - | - |  | - |
|  |  |  |  |  | ✓ |
| service_relocated_descriptor | 0x0B | - |  | - |  |  | - | - | - |  | - |
|  |  | ✓ |  | ✓ |
| XAIT_PID_descriptor (see ETSI TS 102 727 [i.2]) | 0x0C |  |  |  | - |  | - | - | - |  | - |
|  |  | ✓ |
| C2_delivery_system_descriptor | 0x0D |  |  | - | - |  | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| DTS-HD_descriptor (see annex G) | 0x0E | - |  | - | - |  | - | - |  |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| DTS_Neural_descriptor (see annex L) | 0x0F | - |  | - | - |  | - | - |  |  | - |
|  |  |  |  |  | ✓ |  | ✓ |
| video_depth_range_descriptor | 0x10 | - |  | - |  |  |  | - | - |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| T2MI_descriptor | 0x11 | - |  | - | - |  | - | - |  |  | - |
| reserved for future use | 0x12 |
|  |  | ✓ |  | ✓ | ✓ |  | ✓ |  | ✓ |  | ✓ |
| URI_linkage_descriptor | 0x13 |  |  |  |  |  |  | - |
|  |  | ✓ |  | ✓ | ✓ |  | ✓ |
| CI_ancillary_data_descriptor (see ETSI | 0x14 |  |  |  |  |  |  | - | - |  | - |
| TS 103 205 [i.3]) |
|  |  |  |  |  |  |  |  |  | ✓ |
| AC-4_descriptor (see annex D) | 0x15 | - |  | - | - |  | - | - |  |  | - |
|  |  | ✓ |
| C2_bundle_delivery_system_descriptor | 0x16 |  |  | - | - |  | - | - | - |  | - |
|  |  | ✓ |
| S2X_satellite_delivery_system_descriptor | 0x17 |  |  | - | - |  | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| protection_message_descriptor (see ETSI | 0x18 | - |  | - | - |  | - | - |  |  | - |
| TS 102 809 [25]) |
|  |  |  |  |  |  |  |  |  | ✓ |
| audio_preselection_descriptor | 0x19 | - |  | - | - |  | - | - |  |  | - |
| reserved for future use | 0x1A to 0x1F |
|  |  |  |  |  |  |  |  |  | ✓ |
| TTML_subtitling_descriptor (see ETSI | 0x20 | - |  | - | - |  | - | - |  |  | - |
| EN 303 560 [12]) |
|  |  |  |  |  |  |  |  |  | ✓ |
| DTS-UHD_descriptor (see annex G) | 0x21 | - |  | - | - |  | - | - |  |  | - |
|  |  | ✓ |  | ✓ | ✓ |
| service_prominence_descriptor | 0x22 |  |  |  |  |  | - | - | - |  | - |
|  |  |  |  |  | ✓ |  | ✓ |  | ✓ |
| vvc_subpictures_descriptor | 0x23 | - |  | - |  |  |  | - |  |  | - |
|  |  | ✓ |
| S2Xv2_satellite_delivery_system_descriptor | 0x24 |  |  | - | - |  | - | - | - |  | - |
| reserved for future use | 0x25 to 0x7F |
| user defined | 0x80 to 0xFF |
| NOTE: Only found in Partial Transport Streams. |

