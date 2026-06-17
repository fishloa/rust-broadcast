## Table 2 — Allocation of table_id values
_§5.1.4.2, PDF pp. 25-25_

| table_id | Description |
|---|---|
| 0x00 | program_association_section |
| 0x01 | conditional_access_section |
| 0x02 | program_map_section |
| 0x03 | transport_stream_description_section |
| 0x04 to 0x3F | reserved |
| 0x40 | network_information_section - actual network |
| 0x41 | network_information_section - other network |
| 0x42 | service_description_section - actual DVB transport stream |
| 0x43 to 0x45 | reserved for future use |
| 0x46 | service_description_section - other DVB transport stream |
| 0x47 to 0x49 | reserved for future use |
| 0x4A | bouquet_association_section |
| 0x4B | update notification table section (ETSI TS 102 006 [20]) |
| 0x4C | IP/MAC_notification_section (ETSI EN 301 192 [3] - see note 2) |
| 0x4D | satellite_access_section |
| 0x4E | event_information_section - actual DVB transport stream, present/following |
| 0x4F | event_information_section - other DVB transport stream, present/following |
| 0x50 to 0x5F | event_information_section - actual DVB transport stream, schedule |
| 0x60 to 0x6F | event_information_section - other DVB transport stream, schedule |
| 0x70 | time_date_section |
| 0x71 | running_status_section |
| 0x72 | stuffing_section |
| 0x73 | time_offset_section |
| 0x74 | application information section (ETSI TS 102 812 [26]) |
| 0x75 | container section (ETSI TS 102 323 [21]) |
| 0x76 | related content section (ETSI TS 102 323 [21]) |
| 0x77 | content identifier section (ETSI TS 102 323 [21]) |
| 0x78 | MPE-FEC section (ETSI EN 301 192 [3]) |
| 0x79 | resolution provider notification section (ETSI TS 102 323 [21]) |
| 0x7A | MPE-IFEC section (ETSI TS 102 772 [23]) |
| 0x7B | protection message section (ETSI TS 102 809 [25]) |
| 0x7C | downloadable font info section (ETSI EN 303 560 [12] - see note 2) |
| 0x7D | reserved for future use |
| 0x7E | discontinuity_information_section |
| 0x7F | selection_information_section |
| 0x80 to 0xFE | user defined |
| 0xFF | reserved (see note 1) |

