# Table 16: Fixed Link Functions of Packets X/27/4 and X/27/5

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Packet | Link | Function | Fixed Usage | Optional Usage  |
| --- | --- | --- | --- | --- |
|   | 0 | GPOP (Global Public Object Page) | Level 2.5 | Level 3.5  |
|   | 1 | POP (Public Object Page) | Level 2.5 | Level 3.5  |
|  X/27/4 | 2 | GDRCS (Global DRCS Page) | Level 2.5 | Level 3.5  |
|   | 3 | DRCS (Normal DRCS Page) | Level 2.5 | Level 3.5  |
|   | 4 | Defined by Link Function bits |  | Level 3.5  |
|   | 5 | Defined by Link Function bits |  | Level 3.5  |
|   | 0 | Defined by Link Function bits |  | Level 3.5  |
|   | 1 | Defined by Link Function bits |  | Level 3.5  |
|  X/27/5 | 2 | Reserved |  |   |
|   | 3 | Reserved |  |   |
|   | 4 | Reserved |  |   |
|   | 5 | Reserved |  |   |
|  NOTE: Duplicate settings are invalid, i.e. two GPOP links cannot be specified.  |   |   |   |   |

## 9.6.3 Packets X/27/4 to X/27/7 - Format 2 - for compositional linking in data broadcasting applications

Format 2 packets X/27 have valid designation codes of 0100 and 0111. The packets define compositional links in Page Format - CA data broadcasting applications according to EN 300 708 [2] clause 5.

The overall structure of Format 2 packets is the same as that shown for Format 1 in figure 14, but the detailed coding is different.

Byte 6 is the designation code, coded Hamming 8/4. Bytes 7 to 42 are arranged as 6 groups of $3 + 3$ bytes, each sub-group of 3 bytes being one Hamming 24/18 coded triplet. Each group of 6 bytes defines a linked page address, the groups being numbered 0 to 5 in order of transmission. Bytes 43 to 45 are also Hamming 24/18 coded but the data bits are all set to '0'.

Each linked page address of $3 + 3$ bytes contains 36 data bits:

Relative magazine number: 3 bits;

Page number: 8 bits;

Page sub-code: 13 bits;

Link control data: 12 bits.

The mapping of these functions within a two triplet group, and the allocation of links to triplets, is shown in table 17.
