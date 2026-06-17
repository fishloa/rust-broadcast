# Table 7: Page function and page coding bits (packets X/28/0 Format 2 and X/28/2)

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |   |   |   |   |   |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
|  1 | 1-8 | Page Function These bits define the function of the data in packets X/1 to X/25 of the associated page when bits 9 to 14 of this triplet are all set to '0'.  |   |   |   |   |   |   |   |   |   |   |
|   |  |  | Bit |   |   |   |   |   |   |  |  |   |
|   |  |  | 8 | 7 | 6 | 5 | 4 | 3 | 2 | 1 | Page Function |   |
|   |  |  | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | Basic Level 1 Teletext page with standard character position and row format |   |
|   |  |  | 1 | 0 | 0 | 0 | 0 | 1 | 0 | 0 | Reformatted data |   |
|   |  |  | 1 | 0 | 0 | 0 | 0 | 1 | 0 | 1 | Terminal equipment addressing page |   |
|   |  | Other combinations are reserved for future use.  |   |   |   |   |   |   |   |   |   |   |
|  1 | 9-14 | Set to '0'  |   |   |   |   |   |   |   |   |   |   |
|  1 | 15-18 | Page Coding These bits define the coding of packets X/1 to X/25 of the associated page.  |   |   |   |   |   |   |   |   |   |   |
|   |  |  | Bit |   |   |   |  |   |   |   |   |   |
|   |  |  | 18 | 17 | 16 | 15 | Page Coding |   |   |   |   |   |
|   |  |  | 0 | 0 | 0 | 0 | All 8-bit bytes, each comprising 7 data bits and 1 odd parity bit. |   |   |   |   |   |
|   |  |  | 0 | 0 | 0 | 1 | All 8-bit bytes, each comprising 8 data bits. |   |   |   |   |   |
|   |  |  | 0 | 0 | 1 | 0 | Per packet: One 8-bit byte coded Hamming 8/4, followed by thirteen groups of three 8-bit bytes coded Hamming 24/18. All packets coded in this way. |   |   |   |   |   |
|   |  |  | 0 | 0 | 1 | 1 | All 8-bit bytes, each code Hamming 8/4. |   |   |   |   |   |
|   |  | Other combinations are reserved for future use.  |   |   |   |   |   |   |   |   |   |   |
|  2-13 | 1-18 | Reserved  |   |   |   |   |   |   |   |   |   |   |

## 9.4.4 Packet X/28/1

A packet X/28 with a designation code value of 0001 may be transmitted as part of any page at any presentation Level. When associated with a Level 1 Teletext page the packet is used for:

- G0 and G1 character designation (but only for compatibility with some existing Level 1 and 1.5 decoders designed to earlier Teletext specifications);
- DCLUT4 for global 12x10x2 DRCS mode characters };
- DCLUT4 for normal 12x10x2 DRCS mode characters } at Level 3.5;
- DCLUT16 for global 12x10x4 and 6x5x4 DRCS mode characters } (see clauses 14.2.2 to 14.2.4);
- DCLUT16 for normal 12x10x4 and 6x5x4 DRCS mode characters }.

The coding shown in table 8 applies when the packet forms part of a basic Level 1 page.
