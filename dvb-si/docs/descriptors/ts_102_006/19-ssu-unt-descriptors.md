## Table 19 — SSU UNT descriptors
_§9.5.2, PDF pp. 25-25_

| Descriptor | Tag Value | Allowed in Loop: Common | Target | Operational |
|---|---|---|---|---|
| Defined by the present document | 0x00 to 0x3F | | | |
| reserved | 0x00 | | | |
| scheduling_descriptor | 0x01 | * | | * |
| update_descriptor | 0x02 | * | | * |
| ssu_location_descriptor | 0x03 | * | | * |
| message_descriptor | 0x04 | * | | * |
| ssu_event_name_descriptor | 0x05 | * | | * |
| target_smartcard_descriptor | 0x06 | | * | |
| target_MAC_address_descriptor | 0x07 | | * | |
| target_serial_number_descriptor | 0x08 | | * | |
| target_IP_address_descriptor | 0x09 | | * | |
| target_IPv6_address_descriptor | 0x0A | | * | |
| ssu_subgroup_association_descriptor | 0x0B | | | * |
| enhanced_message_descriptor | 0x0C | * | | * |
| ssu_uri_descriptor | 0x0D | * | | * |
| In the scope of DVB-SI [4] | 0x40 to 0x7F | | | |
| telephone_descriptor | 0x57 | * | | * |
| private_data_specifier_descriptor | 0x5F | * | * | * |
| user private | 0x80 to 0xFE | | | |
| reserved | 0xFF | | | |

