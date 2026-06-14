# Table 3: Page function and page coding bits (packets X/28/0 Format 1, X/28/3 and X/28/4)

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- |
|  1 | 1-4 | Page Function These bits define the function of the data in packets X/1 to X/25 of the associated page.  |   |   |   |   |   |
|   |  | Bit |   |   |   |  |   |
|   |  | 4 | 3 | 2 | 1 | Page Function |   |
|   |  | 0 | 0 | 0 | 0 | Basic Level 1 Teletext page (LOP) |   |
|   |  | 0 | 0 | 0 | 1 | Data broadcasting page coded according to EN 300 708 [2], clause 4 |   |
|   |  | 0 | 0 | 1 | 0 | Global Object definition page (GPOP) - (see clause 10.5.1) |   |
|   |  | 0 | 0 | 1 | 1 | Normal Object definition page (POP) - (see clause 10.5.1) |   |
|   |  | 0 | 1 | 0 | 0 | Global DRCS downloading page (GDRCS) - (see clause 10.5.2) |   |
|   |  | 0 | 1 | 0 | 1 | Normal DRCS downloading page (DRCS) - (see clause 10.5.2) |   |
|   |  | 0 | 1 | 1 | 0 | Magazine Organization table (MOT) - (see clause 10.6) |   |
|   |  | 0 | 1 | 1 | 1 | Magazine Inventory page (MIP) - (see clause 11.3) |   |
|   |  | 1 | 0 | 0 | 0 | Basic TOP table (BTT) |   |
|   |  | 1 | 0 | 0 | 1 | Additional Information Table (AIT) } (see clause 11.2) |   |
|   |  | 1 | 0 | 1 | 0 | Multi-page table (MPT) |   |
|   |  | 1 | 0 | 1 | 1 | Multi-page extension table (MPT-EX) |   |
|   |  | 1 | 1 | 0 | 0 | Page contain trigger messages defined according to [8] |   |
|   |  | Other combinations are reserved for future use.  |   |   |   |   |   |
|  1 | 5-7 | Page Coding These bits define the coding of packets X/1 to X/25 of the associated page.  |   |   |   |   |   |
|   |  | Bit |   |   |   |  |   |
|   |  | 7 | 6 | 5 | Page Coding |   |   |
|   |  | 0 | 0 | 0 | All 8-bit bytes, each comprising 7 data bits and 1 odd parity bit. |   |   |
|   |  | 0 | 0 | 1 | All 8-bit bytes, each comprising 8 data bits. |   |   |
|   |  | 0 | 1 | 0 | Per packet: One 8-bit byte coded Hamming 8/4, followed by thirteen groups of three 8-bit bytes coded Hamming 24/18. All packets coded in this way. |   |   |
|   |  | 0 | 1 | 1 | All 8-bit bytes, each code Hamming 8/4. |   |   |
|   |  | 1 | 0 | 0 | Per packet: Eight 8-bit bytes coded Hamming 8/4, followed by twelve 8-bit bytes coded 7 data bits and 1 odd parity bit. This sequence is then repeated for the remaining 20 bytes. All packets coded in this way. |   |   |
|   |  | 1 | 0 | 1 | Per packet: First 8-bit byte coded Hamming 8/4. The data bits from this byte define the coding of the remaining 39 bytes of this packet only, according to the first five entries in this table. |   |   |
|   |  | Other combinations are reserved for future use.  |   |   |   |   |   |

## 9.4.2.2 Coding for basic Level 1 Teletext pages

When the Page Function bits (triplet 1, bits 1 to 4) indicate a basic Level 1 Teletext page (code 0000), the remaining bits of the packet define the following Level 2.5 and 3.5 presentation related data:

- Default character sets;
- Size and position of any side-panels;
- Colour map entry coding for CLUTs 2 and 3;
- Default screen and row colours;
- Colour table re-mapping of the foreground and background colours of the basic Level 1 page.

The coding is shown in table 4. The same coding also applies to packets X/28/4 except that they redefine CLUTs 0 and 1 instead of CLUTs 2 and 3.

Where packets 28/0 and 28/4 are both transmitted as part of a page, packet 28/0 takes precedence over 28/4 for all but the colour map entry coding.
