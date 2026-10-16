## Table 19 — INT descriptors
_§8.4.5.1, PDF pp. 31-31_

Verified against the rendered page 31 of ETSI EN 301 192 V1.7.1 (2021-08). The earlier extraction lost the Platform/Target/Operational column alignment; the `*` cells are:

| Descriptor | Tag Value | Allowed in Loop: Platform | Target | Operational |
|---|---|---|---|---|
| reserved | 0x00 | | | |
| target_smartcard_descriptor | 0x06 | | * | |
| target_MAC_address_descriptor | 0x07 | | * | |
| target_serial_number_descriptor | 0x08 | | * | |
| target_IP_address_descriptor | 0x09 | | * | |
| target_IPv6_address_descriptor | 0x0A | | * | |
| IP/MAC_platform_name_descriptor | 0x0C | * | | |
| IP/MAC_platform_provider_name_descriptor | 0x0D | * | | |
| target_MAC_address_range_descriptor | 0x0E | | * | |
| target_IP_slash_descriptor | 0x0F | | * | |
| target_IP_source_slash_descriptor | 0x10 | | * | |
| target_IPv6_slash_descriptor | 0x11 | | * | |
| target_IPv6_source_slash_descriptor | 0x12 | | * | |
| IP/MAC_stream_location_descriptor | 0x13 | * | | * |
| ISP_access_mode_descriptor | 0x14 | * | | * |
| IP/MAC_generic_stream_location_descriptor | 0x15 | * | | * |
| telephone_descriptor | 0x57 | * | | * |
| private_data_specifier_descriptor | 0x5F | * | * | * |
| user private | 0x80 to 0xFE | | | |
| reserved | 0xFF | | | |

Descriptors from the DVB SI range (0x40 to 0x7F) shall have their standard semantics as defined in ETSI EN 300 468 [2]. Equally MPEG descriptors in the range 0x00 to 0x3F cannot be used in the INT. Descriptor tags 0x00 to 0x3F share a common descriptor name space with the UNT descriptors (ETSI TS 102 006 [18]).
