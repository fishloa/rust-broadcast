## Table 12 — Possible locations of descriptors
_§6.1, PDF pp. 52-53_

| Descriptor | Tag | NIT |  | BAT |  | SDT | EIT | TOT | PMT | SIT |
|---|---|---|---|---|---|---|---|---|---|---|
|  | value |  |  |  |  |  |  |  |  | (see note 1) |
|  |  | ✓ |
| network_name_descriptor | 0x40 |  |  | - |  | - | - | - | - | - |
|  |  | ✓ |  | ✓ |
| service_list_descriptor | 0x41 |  |  |  |  | - | - | - | - | - |
|  |  | ✓ |  | ✓ |  | ✓ | ✓ |  |  | ✓ |
| stuffing_descriptor | 0x42 |  |  |  |  |  |  | - | - |
|  |  | ✓ |
| satellite_delivery_system_descriptor | 0x43 |  |  | - |  | - | - | - | - | - |
|  |  | ✓ |
| cable_delivery_system_descriptor | 0x44 |  |  | - |  | - | - | - | - | - |
| Descriptor | Tag | NIT | BAT | SDT | EIT | TOT | PMT |  | SIT |
|  | value |  |  |  |  |  |  |  | (see note 1) |
|  |  |  |  |  |  |  | ✓ |
| VBI_data_descriptor | 0x45 | - | - | - | - | - |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| VBI_teletext_descriptor | 0x46 | - | - | - | - | - |  |  | - |
|  |  |  | ✓ |  |  |  |  |  | ✓ |
| bouquet_name_descriptor | 0x47 | - |  | - | - | - | - |
|  |  |  |  | ✓ |  |  |  |  | ✓ |
| service_descriptor | 0x48 | - | - |  | - | - | - |
|  |  |  | ✓ | ✓ |  |  |  |  | ✓ |
| country_availability_descriptor | 0x49 | - |  |  | - | - | - |
|  |  | ✓ | ✓ | ✓ | ✓ |  |  |  | ✓ |
| linkage_descriptor | 0x4A |  |  |  |  | - | - |
|  |  |  |  | ✓ |  |  |  |  | ✓ |
| NVOD_reference_descriptor | 0x4B | - | - |  | - | - | - |
|  |  |  |  | ✓ |  |  |  |  | ✓ |
| time_shifted_service_descriptor | 0x4C | - | - |  | - | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| short_event_descriptor | 0x4D | - | - | - |  | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| extended_event_descriptor | 0x4E | - | - | - |  | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| time_shifted_event_descriptor | 0x4F | - | - | - |  | - | - |
|  |  |  |  | ✓ | ✓ |  |  |  | ✓ |
| component_descriptor | 0x50 | - | - |  |  | - | - |
|  |  |  |  | ✓ |  |  | ✓ |  | ✓ |
| mosaic_descriptor | 0x51 | - | - |  | - | - |
|  |  |  |  |  |  |  | ✓ |
| stream_identifier_descriptor | 0x52 | - | - | - | - | - |  |  | - |
|  |  |  | ✓ | ✓ | ✓ |  |  |  | ✓ |
| CA_identifier_descriptor | 0x53 | - |  |  |  | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| content_descriptor | 0x54 | - | - | - |  | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| parental_rating_descriptor | 0x55 | - | - | - |  | - | - |
|  |  |  |  |  |  |  | ✓ |
| teletext_descriptor | 0x56 | - | - | - | - | - |  |  | - |
|  |  |  |  | ✓ | ✓ |  |  |  | ✓ |
| telephone_descriptor | 0x57 | - | - |  |  | - | - |
|  |  |  |  |  |  | ✓ |
| local_time_offset_descriptor | 0x58 | - | - | - | - |  | - |  | - |
|  |  |  |  |  |  |  | ✓ |
| subtitling_descriptor | 0x59 | - | - | - | - | - |  |  | - |
|  |  | ✓ |
| terrestrial_delivery_system_descriptor | 0x5A |  | - | - | - | - | - |  | - |
|  |  | ✓ |
| multilingual_network_name_descriptor | 0x5B |  | - | - | - | - | - |  | - |
|  |  |  | ✓ |
| multilingual_bouquet_name_descriptor | 0x5C | - |  | - | - | - | - |  | - |
|  |  |  |  | ✓ |  |  |  |  | ✓ |
| multilingual_service_name_descriptor | 0x5D | - | - |  | - | - | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| multilingual_component_descriptor | 0x5E | - | - | - |  | - | - |
|  |  | ✓ | ✓ | ✓ | ✓ |  | ✓ |  | ✓ |
| private_data_specifier_descriptor | 0x5F |  |  |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| service_move_descriptor | 0x60 | - | - | - | - | - |  |  | - |
|  |  |  |  |  | ✓ |  |  |  | ✓ |
| short_smoothing_buffer_descriptor | 0x61 | - | - | - |  | - | - |
|  |  | ✓ |
| frequency_list_descriptor | 0x62 |  | - | - | - | - | - |  | - |
|  |  |  |  |  |  |  |  |  | ✓ |
| partial_transport_stream_descriptor | 0x63 | - | - | - | - | - | - |
|  |  |  |  | ✓ | ✓ |  |  |  | ✓ |
| data_broadcast_descriptor | 0x64 | - | - |  |  | - | - |
|  |  |  |  |  |  |  | ✓ |
| scrambling_descriptor | 0x65 | - | - | - | - | - |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| data_broadcast_id_descriptor | 0x66 | - | - | - | - | - |  |  | - |
| transport_stream_descriptor (see note 2) | 0x67 | - | - | - | - | - | - |  | - |
| DSNG_descriptor (see note 2) | 0x68 | - | - | - | - | - | - |  | - |
|  |  |  |  |  | ✓ |
| PDC_descriptor | 0x69 | - | - | - |  | - | - |  | - |
|  |  |  |  |  |  |  | ✓ |
| AC-3_descriptor (see annex D) | 0x6A | - | - | - | - | - |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| ancillary_data_descriptor | 0x6B | - | - | - | - | - |  |  | - |
|  |  | ✓ |
| cell_list_descriptor | 0x6C |  | - | - | - | - | - |  | - |
|  |  | ✓ |
| cell_frequency_link_descriptor | 0x6D |  | - | - | - | - | - |  | - |
|  |  |  |  | ✓ |
| announcement_support_descriptor | 0x6E | - | - |  | - | - | - |  | - |
|  |  |  |  |  |  |  | ✓ |
| application_signalling_descriptor (see ETSI | 0x6F | - | - | - | - | - |  |  | - |
| TS 102 809 [25]) |
|  |  |  |  |  |  |  | ✓ |
| adaptation_field_data_descriptor | 0x70 | - | - | - | - | - |  |  | - |
|  |  |  |  | ✓ |
| service_identifier_descriptor (see ETSI | 0x71 | - | - |  | - | - | - |  | - |
| TS 102 812 [26]) |
|  |  |  |  | ✓ |
| service_availability_descriptor | 0x72 | - | - |  | - | - | - |  | - |
|  |  | ✓ | ✓ | ✓ |
| default_authority_descriptor (see ETSI | 0x73 |  |  |  | - | - | - |  | - |
| TS 102 323 [21]) |
|  |  |  |  |  |  |  | ✓ |
| related_content_descriptor (see ETSI | 0x74 | - | - | - | - | - |  |  | - |
| TS 102 323 [21]) |
|  |  |  |  |  | ✓ |
| TVA_id_descriptor (see ETSI TS 102 323 [21]) | 0x75 | - | - | - |  | - | - |  | - |
|  |  |  |  |  | ✓ |
| content_identifier_descriptor (see ETSI | 0x76 | - | - | - |  | - | - |  | - |
| TS 102 323 [21]) |
|  |  | ✓ |
| time_slice_fec_identifier_descriptor | 0x77 |  | - | - | - | - | - |  | - |
| (see note 3) |
|  |  |  |  |  |  |  | ✓ |
| ECM_repetition_rate_descriptor (ETSI | 0x78 | - | - | - | - | - |  |  | - |
| EN 301 192 [3]) |
|  |  | ✓ |
| S2_satellite_delivery_system_descriptor | 0x79 |  | - | - | - | - | - |  | - |
|  |  |  |  |  |  |  | ✓ |
| enhanced_AC-3_descriptor (see annex D) | 0x7A | - | - | - | - | - |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| DTS_descriptor (see annex G) | 0x7B | - | - | - | - | - |  |  | - |
|  |  |  |  |  |  |  | ✓ |
| AAC_descriptor (see annex H) | 0x7C | - | - | - | - | - |  |  | - |
|  |  | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |  | ✓ |
| XAIT_location_descriptor (ETSI TS 102 727 [i.2]) | 0x7D |
|  |  | ✓ | ✓ | ✓ | ✓ |
| FTA_content_management_descriptor | 0x7E |  |  |  |  | - | - |  | - |
|  |  | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |  | ✓ |
| extension_descriptor (see note 4) | 0x7F |
| user defined | 0x80 to 0xFE |
| reserved for future use | 0xFF |

