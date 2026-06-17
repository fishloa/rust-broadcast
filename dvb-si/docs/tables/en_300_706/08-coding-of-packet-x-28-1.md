# Table 8: Coding of Packet X/28/1

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-2 | Set to '00'. (see note)  |
|  1 | 3-9 | Character Set Code for G0 Table. (see note)  |
|  1 | 10 | Set to '0'. (see note)  |
|  1 | 11-17 | Character Set Code for G1 Table. (see note)  |
|  1 | 18 | Set to '0'. (see note)  |
|  2 | 1-18 | DCLUT4 for Global 12x10x2 DRCS Mode Characters  |
|  3 | 1-2 | 4 data words of 5 bits each to define the DCLUT for use with global 12x10x2 DRCS. The transmission order is least significant bit first.  |
|  3 | 3-18 | DCLUT4 for Normal 12x10x2 DRCS Mode Characters  |
|  4 | 1-4 | 4 data words of 5 bits each to define the DCLUT for use with normal 12x10x2 DRCS. The transmission order is least significant bit first.  |
|  4 | 5-18 | DCLUT16 for Global 12x10x4 and 6x5x4 DRCS Mode Characters  |
|  5-7 | 1-18 | 16 data words of 5 bits each to define the DCLUT for use with global 12x10x4 and 6x5x4 DRCS. The transmission order is least significant bit first.  |
|  8 | 1-12 | 16 data words of 5 bits each to define the DCLUT for use with global 12x10x4 and 6x5x4 DRCS. The transmission order is least significant bit first.  |
|  8 | 13-18 | DCLUT16 for Normal 12x10x4 and 6x5x4 DRCS Mode Characters  |
|  9-12 | 1-18 | 16 data words of 5 bits each to define the DCLUT for use with global 12x10x4 and 6x5x4 DRCS. The transmission order is least significant bit first.  |
|  13 | 1-2 | 16 data words of 5 bits each to define the DCLUT for use with global 12x10x4 and 6x5x4 DRCS. The transmission order is least significant bit first.  |
|  13 | 3-18 | Reserved for future use  |
|  NOTE: The function of these bits is defined by earlier specifications and is retained for compatibility with existing Level 1 and 1.5 decoders designed to them. They are not intended for use by Level 2.5 and 3.5 decoders designed to the present document.  |   |   |

## 9.4.5 Packet X/28/2

A packet X/28 with a designation code value of 0010 may be transmitted as part of any page at any presentation Level. It is used to carry a Page Key for descrambling purposes in certain data broadcasting applications, (see EN 300 708 [2], clause 5.4.2). The first triplet is coded in an identical manner to a packet X/28/0 Format 2, as shown in table 7.

## 9.4.6 Packet X/28/3

A packet X/28 with a designation code value of 0011 may be transmitted as part of a DRCS downloading page at presentation Levels 2.5 and 3.5. The first 7 data bits of the packet define the function and the coding of packets X/1 to X/25 of the associated page according to clause 9.4.2.1.

The coding of table 9 applies to the remaining data bits of the packet when the Page Function bits (triplet 1, bits 1 to 4) indicate a global or normal DRCS downloading page (codes 0100 and 0101). The type of DRCS character defined by each pattern transfer unit (PTU) transmitted via packets X/1 to X/24 is specified.
