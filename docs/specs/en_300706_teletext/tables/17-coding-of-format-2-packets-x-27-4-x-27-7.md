# Table 17: Coding of Format 2 packets X/27/4 - X/27/7

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Data Bits | Function  |   |   |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
|  1-2 | 1-18 | Link 0  |   |   |   |   |   |   |   |
|  1 | 1-10 | Link Control Data (part 1) If bit 1 of triplet 2 = 1 or bit 11 of triplet 1 = 1, the function of bits 1 - 10 is reserved for future use. If bit 1 of triplet 2 = 0 and bit 11 of this triplet = 0, the following interpretation applies to these bits:  |   |   |   |   |   |   |   |
|   |  |  | Bit 10 | Bit 9 | Link Type |   |   |   |   |
|   |  |  | 0 | 0 | Linked pages, not chained |   |   |   |   |
|   |  |  | 0 | 1 | Linked pages, chained, start of chain |   |   |   |   |
|   |  |  | 1 | 0 | Linked pages, chained, end of chain |   |   |   |   |
|   |  |  | 1 | 1 | Linked pages, chained, within a chain |   |   |   |   |
|  |   |   |   |   |   |   |   |   |   |
|   |  |  | Bit 8 | Bit 7 | Page Coding |   |   |   |   |
|   |  |  | 0 | 0 | Linked page data format, 7 bits plus odd parity |   |   |   |   |
|   |  |  | 0 | 1 | Interpretation reserved, bits 1 to 6 are also reserved |   |   |   |   |
|   |  |  | 1 | 0 | Interpretation reserved, bits 1 to 6 are also reserved |   |   |   |   |
|   |  |  | 1 | 1 | Linked page contains data in 8 bit format |   |   |   |   |
|  |   |   |   |   |   |   |   |   |   |
|  1 | 1-10 | Bit |   |   |   | Page Function |   |   |   |
|   | (continued) | 6 | 5 | 4 | 3 | 2 | 1 |  |   |
|   |  | 0 | 0 | 0 | 0 | 0 | 0 | Page in standard format |   |
|   |  | 0 | 0 | 0 | 1 | 0 | 1 | Pseudo page for reformatted data |   |
|   |  | 0 | 0 | 0 | 1 | 1 | 0 | Pseudo page for page format extension |   |
|   |  | 1 | 1 | 1 | 1 | 1 | 1 | No linked page; page address FF:3F7F transmitted |   |
|   |  | Other values are reserved  |   |   |   |   |   |   |   |
|  1 | 11 | Link Control Data (part 2) When set to '0', bits 1-10 have the functions described above When set to '1', the interpretation of bits 1-10 is reserved for future use  |   |   |   |   |   |   |   |
|  1 | 12-14 | Relative Magazine Number (LSB - MSB). These bits change the magazine number from that in byte 4 of this packet X/27. Setting any of these bits to '1' complements the corresponding magazine bit.  |   |   |   |   |   |   |   |
|  1 | 15-18 | Page Number Tens (LSB - MSB)  |   |   |   |   |   |   |   |
|  2 | 1 | Link Control Data (part 3) When set to '0', bits 1-11 of triplet 1 have the functions described above. When set to '1', the interpretation of bits 1-11 of triplet 1 is reserved  |   |   |   |   |   |   |   |
|  2 | 2-5 | Page Number Units (LSB - MSB)  |   |   |   |   |   |   |   |
|  2 | 6-7 | Page sub-code - S4 (LSB - MSB)  |   |   |   |   |   |   |   |
|  2 | 8-11 | Page sub-code - S3 (LSB - MSB)  |   |   |   |   |   |   |   |
|  2 | 12-14 | Page sub-code - S2 (LSB - MSB)  |   |   |   |   |   |   |   |
|  2 | 15-18 | Page sub-code - S1 (LSB - MSB)  |   |   |   |   |   |   |   |
|  3-4 | 1-18 | Link 1, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |
|  5-6 | 1-18 | Link 2, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |
|  7-8 | 1-18 | Link 3, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |
|  9-10 | 1-18 | Link 4, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |
|  11-12 | 1-18 | Link 5, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |
|  13 | 1-18 | Set to '0'  |   |   |   |   |   |   |   |

When no particular page number is to be specified, the page number FF is transmitted. When no particular page sub-code is to be specified, the page sub-code 3F7F is transmitted. When the page address FF:3F7F is transmitted, no page is specified and the link control data bits are set to '1'.



# 9.7 General Coding of packets 30 and 31

For packets with addresses 30 and 31, the magazine value represents an additional channel identifier.

These packets can be used to carry information unrelated to, and completely independent of, any accompanying service organized as magazines of pages. They can be inserted at any point within the transmission. Details on their use to provide independent data services are given in EN 300 708 [2] clauses 6 and 7.

# 9.8 Broadcast Service Data Packets

## 9.8.1 Packet 8/30 Format 1

Packets 8/30 Format 1 have designation code values of 0000 or 0001. They carry broadcast service data relating to the TV channel, including:

- multiplexed transmission flag;
- initial Teletext page number;
- network identification;
- current time and date;
- status display.

![img-3.jpeg](img-3.jpeg)
Figure 15: Coding of Packet 8/30 Format 1

The coding of bytes 7 to 45 shown in table 18 applies when the designation value is 0000 or 0001.
