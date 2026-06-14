# Table 6: Coding of Packet X/28/0 Format 1 for other types of pages

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-4 | Page Function - defined according to clause 9.4.2.1.  |
|  1 | 5-7 | Page Coding - defined according to clause 9.4.2.1.  |
|  1 | 8-18 | Set to 11111111100 (bits 8 to 18). This value is chosen to ensure existing data broadcasting decoders, designed according to EN 300 708 [2], ignore this type of page.  |
|  2-13 | 1-18 | Reserved for future use.  |

# 9.4.3 Packet X/28/0 - Format 2

A packet X/28/0 Format 2 is used in data broadcasting applications as part of the Page Format - CA protocol defined in EN 300 708 [2] clause 5. The first 8 data bits of the packet define the function of the associated page, and bits 15 to 18 define the coding of packets X/1 to X/25, as shown in table 7. This coding scheme is also used for the first triplet of packets X/28/2.
