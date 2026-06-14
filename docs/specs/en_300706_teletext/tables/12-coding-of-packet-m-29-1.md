# Table 12: Coding of Packet M/29/1

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-18 | Character Set Codes for G0 and G1 Tables. As clause 9.4.4.  |
|  2 | 1-18 | DCLUT4 for Global 12x10x2 DRCS Mode Characters.  |
|  3 | 1-2 | As clause 9.4.4.  |
|  3 | 3-18 | DCLUT4 for Normal 12x10x2 DRCS Mode Characters.  |
|  4 | 1-4 | As clause 9.4.4.  |
|  4 | 5-18 | DCLUT16 for Global 12x10x4 and 6x5x4 DRCS Mode Characters. As clause 9.4.4.  |
|  5-7 | 1-18 |   |
|  8 | 1-12 |   |
|  8 | 13-18 | DCLUT16 for Normal 12x10x4 and 6x5x4 DRCS Mode Characters. As clause 9.4.4.  |
|  9-12 | 1-18 |   |
|  13 | 1-2 |   |
|  13 | 3-18 | Reserved for future use.  |



## 9.5.3 Packet M/29/4

The coding of the bits applicable to character set designation, side-panels, the CLUT, default row and screen colours, colour table re-mapping and black background substitution in packets X/28/4 is also used in packets M/29/4. This data applies to all basic Level 1 pages in magazine M but is overridden for a particular page if a packet X/28/4 exists for that page. Where M/29/0 and M/29/4 are transmitted for the same magazine, M/29/0 takes precedence over M/29/4.
