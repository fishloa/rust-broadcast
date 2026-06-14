# Table 9: Coding of Packet X/28/3 for DRCS Downloading Pages

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- |
|  1 | 1-7 | Coded according to clause 9.4.2.1. Page Function = 0100 or 0101 (Global or normal DRCS downloading page) Page Coding = 000 (7 bits plus odd parity)  |   |   |   |   |   |
|  1 | 8-18 | Reserved for future use.  |   |   |   |   |   |
|  2-11 | 1-18 | DRCS Downloading Mode Invocation  |   |   |   |   |   |
|  12 | 1-12 | The downloading mode of DRCS characters at Level 3.5 are specified individually for each character. These 192 data bits are used to transmit 48 Mode Identification codes, each comprising 4 bits. One value is assigned to each Pattern Transfer Unit (PTU) of 20 bytes.  |   |   |   |   |   |
|   |  | Bits |   |   | DRCS |  |   |
|   |  | (MSB LSB) |   |   | Mode | Resolution or Function |   |
|   |  | 0 | 0 | 0 | 0 | 12 x 10 x 1 |   |
|   |  | 0 | 0 | 0 | 1 | 12 x 10 x 2 |   |
|   |  | 0 | 0 | 1 | 0 | 12 x 10 x 4 |   |
|   |  | 0 | 0 | 1 | 1 | 6 x 5 x 4 |   |
|   |  | 1 | 1 | 1 | 0 | Subsequent PTU of a Mode 1 or 2 character |   |
|   |  | 1 | 1 | 1 | 1 | No data for the corresponding character |   |
|   |  | Other values are reserved. Where a DRCS character is defined by more than one PTU, the appropriate mode value is used for the first PTU and subsequent PTUs are coded 1110.  |   |   |   |   |   |
|  12 | 13-18 | Reserved for future use  |   |   |   |   |   |
|  13 | 1-18 |   |   |   |   |   |   |

## 9.4.7 Packet X/28/4

A packet X/28 with a designation code value of 0100 may be transmitted as part of any page at presentation Level 3.5. The first 7 data bits of the packet define the function and the coding of packets X/1 to X/25 of the associated page according to clause 9.4.2.1.

When the Page Function bits (triplet 1, bits 1 to 4) indicate a Level 1 Teletext page (code 0000), the remaining bits of the packet define the following Level 2.5 and 3.5 presentation related data:

- Default character sets;
- Size and position of any side-panels;
- Colour map entry coding for CLUTs 0 and 1;
- Default screen and row colours.

Colour table re-mapping of the foreground and background colours of the Level 1 page.

The coding is shown in table 10. It is identical to packets X/28/0 Format 1 except that it redefines CLUTs 2 and 3 instead of CLUTs 0 and 1.

Where packets 28/0 and 28/4 are both transmitted as part of a page, packet 28/0 takes precedence over 28/4 for all but the colour map entry coding.
