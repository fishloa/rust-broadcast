# C(P)SIG ⇔ (P)SIG interface

_ETSI TS 103 197 V1.5.1 §8 — Custom (Program) Specific Information Generator to
(Program) Specific Information Generator. Message-based protocol; same 5-byte
header + TLV framing as ECMG⇔SCS / EMMG⇔MUX (already implemented), new
`message_type` (0x03xx) and `parameter_type` (0x01xx) ranges._

## Message types (§8, message-type summary table)

| Value | Message |
|---|---|
| 0x0301 | channel_setup |
| 0x0302 | channel_status |
| 0x0303 | channel_test |
| 0x0304 | channel_close |
| 0x0305 | channel_error |
| 0x0311 | stream_setup |
| 0x0312 | stream_status |
| 0x0313 | stream_test |
| 0x0314 | stream_close |
| 0x0315 | stream_close_request |
| 0x0316 | stream_close_response |
| 0x0317 | stream_error |
| 0x0318 | stream_service_change |
| 0x0319 | stream_trigger_enable_request |
| 0x031A | stream_trigger_enable_response |
| 0x031B | trigger |
| 0x031C | table_request |
| 0x031D | table_response |
| 0x031E | descriptor_insert_request |
| 0x031F | descriptor_insert_response |
| 0x0320 | PID_provision_request |
| 0x0321 | PID_provision_response |

`0x0306`–`0x0310` and `0x0322`–`0x0400` are DVB reserved; `0x8000`–`0xFFFF` user defined.

## Table 36 — `parameter_type` values (C(P)SIG ⇔ (P)SIG)

| Value | Parameter | Type/units | Length (bytes) |
|---|---|---|---|
| 0x000D | access_criteria | user defined | variable |
| 0x0100 | bouquet_id | uimsbf | 2 |
| 0x0101 | CA_descriptor_insertion_mode | uimsbf | 1 |
| 0x0102 | custom_CAS_id | uimsbf | 4 |
| 0x0103 | custom_channel_id | uimsbf | 2 |
| 0x0104 | custom_stream_id | uimsbf | 2 |
| 0x0105 | descriptor | per MPEG/DVB | variable |
| 0x0106 | descriptor_insert_status | uimsbf | 1 |
| 0x0107 | duration | uimsbf | 3 |
| 0x0108 | ECM_related_data | — | variable |
| 0x010B | ES_id | uimsbf | 2 |
| 0x010C | event_id | uimsbf | 2 |
| 0x010D | event_related_data | — | variable |
| 0x010E | flow_id | uimsbf | 2 |
| 0x010F | flow_PID | uimsbf | 2 |
| 0x0110 | flow_PID_change_related_data | — | 9 |
| 0x0111 | flow_super_CAS_id | uimsbf | 4 |
| 0x0112 | flow_type | uimsbf | 1 |
| 0x0113 | insertion_delay | tcimsbf (/ms) | 2 |
| 0x0114 | insertion_delay_type | uimsbf | 1 |
| 0x0115 | last_section_indicator | boolean | 1 |
| 0x0116 | location_id | uimsbf | 1 |
| 0x0117 | max_comp_time | uimsbf (/sec) | 2 |
| 0x0118 | max_streams | uimsbf | 2 |
| 0x0119 | MPEG_section | per MPEG/DVB | variable |
| 0x011A | network_id | uimsbf | 2 |
| 0x011B | original_network_id | uimsbf | 2 |
| 0x011C | private_data | user-defined | variable |
| 0x011D | private_data_specifier | uimsbf | 4 |
| 0x011E | (P)SIG_type | uimsbf | 1 |
| 0x011F | segment_number | uimsbf | 1 |
| 0x0120 | service_id | uimsbf | 2 |
| 0x0121 | service_parameters | — | 8 |
| 0x0122 | start_time | bslbf | 5 |
| 0x0123 | stream_change_timestamp | bslbf | 5 |
| 0x0124 | stream_change_type | uimsbf | 1 |
| 0x0125 | table_id | uimsbf | 1 |
| 0x0126 | transaction_id | uimsbf | 2 |
| 0x0127 | transport_stream_id | uimsbf | 2 |
| 0x0128 | trigger_id | uimsbf | 2 |
| 0x0129 | trigger_list | bslbf | 4 |
| 0x012A | trigger_type | uimsbf | 4 |
| 0x012B | PD_related_data | — | variable |
| 0x012C | flow_stream_type | uimsbf | 1 |
| 0x7000 | error_status | (2-byte) | 2 |
| 0x7001 | error_information | user defined | variable |

`0x0109`/`0x010A` and `0x012D`–`0x6FFF` and `0x7002`–`0x7FFF` are DVB reserved;
`0x8000`–`0xFFFF` user defined. (`access_criteria` 0x000D is shared with the ECMG
table.) The wire framing (header + TLV) is identical to the existing interfaces —
only these two new code spaces are added.
