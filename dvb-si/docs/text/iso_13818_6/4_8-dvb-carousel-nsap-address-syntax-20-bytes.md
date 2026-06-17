## Table 4.8 — DVB Carousel NSAP Address syntax (20 bytes)
_§4.7.3.4, PDF p. 35_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `AFI` | 8 | `0x00` | NSAP for private use |
| `Type` | 8 | `0x00` | Object carousel NSAP address |
| `carouselId` | 32 | + | |
| `specifierType` | 8 | `0x01` | IEEE OUI |
| `specifierData` (IEEE OUI) | 24 | `0x00_00_DVB`* | constant for the DVB OUI |
| `transport_stream_id` | 16 | + | |
| `original_network_id` | 16 | + | |
| `service_id` | 16 | + | = MPEG-2 program_number |
| `reserved` | 32 | `0xFFFFFFFF` | |

\* the DVB OUI value; semantics per EN 301 192. Total = 8+8+32+8+24+16+16+16+32 = 160 bits = 20 bytes (matches `serviceDomain_length = 0x14`).

