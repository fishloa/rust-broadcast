# Table 10: Coding of packet X/28/4 for basic Level 1 pages

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-7 | Page Function and Page Coding. As clause 9.4.2.1.  |
|  1 | 8-14 | Default G0 and G2 Character Set Designation and National Option Selection As clause 9.4.2.2.  |
|  1 | 15-18 | Second G0 Set Designation and National Option Selection. As clause 9.4.2.2.  |
|  2 | 1-3  |   |
|  2 | 4 | Left Side Panel. As clause 9.4.2.2.  |
|  2 | 5 | Right Side Panel. As clause 9.4.2.2.  |
|  2 | 6 | Side Panel Status Flag. As clause 9.4.2.2.  |
|  2 | 7-10 | Number of Columns in Side Panels. As clause 9.4.2.2.  |
|  2 | 11-18 | Colour Map Entry Coding for CLUTs 0 and 1.  |
|  3-12 | 1-18 | The bits are organized as 16 data words, each of 12 bits. Each word defines an entry in the Colour Map of clause 12.4, proceeding in transmission order from CLUT 0, entry 0 to CLUT 1, entry 7. Each 12 bit data word contains 4 bits for each primary colour (Red, Green and Blue), in the transmission order: RRRRGGGBBBB, with ascending order of bit significance within each 4 bits. CLUT 1, entry 0 is always "transparent". The corresponding bits for this entry should be ignored by decoders.  |
|  13 | 1-4  |   |
|  13 | 5-9 | Default Screen Colour. As clause 9.4.2.2.  |
|  13 | 10-14 | Default Row Colour. As clause 9.4.2.2.  |
|  13 | 15 | Black Background Colour Substitution. As clause 9.4.2.2.  |
|  13 | 16-18 | Colour Table Re-mapping for use with Spacing Attributes. As clause 9.4.2.2.  |

## 9.5 Magazine-Related Page Enhancement Data Packets

### 9.5.1 Packet M/29/0

The coding of the bits applicable to character set designation, side-panels, the CLUT, default row and screen colours, colour table re-mapping and black background substitution in packets X/28/0 Format 1 is also used in packets M/29/0. This data applies to all basic Level 1 pages in magazine M but is overridden for a particular page if a packet X/28/0 Format 1 exists for that page. Where M/29/0 and M/29/4 are transmitted for the same magazine, M/29/0 takes precedence over M/29/4.
