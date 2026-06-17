# Table 5: Coding of Packet X/28/0 Format 1 for Data Broadcasting Pages

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-4 | Page Function = Data broadcasting page (see clause 9.4.2.1).  |
|  1 | 5-7 | Page Coding - defined according to clause 9.4.2.1.  |
|  1 | 8-18 | Set to 11111111100 (bits 8 to 18). This value is chosen to ensure existing data broadcasting decoders, designed according to EN 300 708 [2], ignore this type of page.  |
|  2-13 | 1-18 | Define by the data broadcasting application.  |

# 9.4.2.4 Coding for other types of page

The coding of table 6 applies to the data bits of a packet X/28/0 Format 1 when the Page Function bits indicate a page other than a basic Level 1 Teletext page (code 0000) or a data broadcasting page (code 0001).
