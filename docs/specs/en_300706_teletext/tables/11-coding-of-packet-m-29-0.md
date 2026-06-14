# Table 11: Coding of Packet M/29/0

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |   |   |   |   |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
|  1 | 1-7 | Packet Function These bits define the application and scope of the data bits in the remainder of this packet.  |   |   |   |   |   |   |   |   |   |
|   |  | Bits |   |   |   |   |   |   |  |  |   |
|   |  | 7 | 6 | 5 | 4 | 3 | 2 | 1 | Packet Function |  |   |
|   |  | 0 | 0 | 0 | 0 | 0 | 0 | 0 | The remaining data bits have an identical coding and function to those of packet X/28/0 Format 1 (see clause 9.4.2) except that here the data applies to all pages in magazine M. |  |   |
|   |  | All other values are reserved for future use.  |   |   |   |   |   |   |   |   |   |
|  1 | 8-14 | Default G0 and G2 Character Set Designation and National Option Selection. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  1 | 15-18 | Second G0 Set Designation and National Option Selection. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  2 | 1-3 |   |   |   |   |   |   |   |   |   |   |
|  2 | 4 | Left Side Panel. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  2 | 5 | Right Side Panel. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  2 | 6 | Side Panel Status Flag. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  2 | 7-10 | Number of Columns in Side Panels. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  2 | 11-18 | Colour Map Entry Coding for CLUTs 2 and 3. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  3-12 | 1-18 |   |   |   |   |   |   |   |   |   |   |
|  13 | 1-4 |   |   |   |   |   |   |   |   |   |   |
|  13 | 5-9 | Default Screen Colour. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  13 | 10-14 | Default Row Colour. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  13 | 15 | Black Background Colour Substitution. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |
|  13 | 16-18 | Colour Table Re-mapping for use with Spacing Attributes. As clause 9.4.2.2.  |   |   |   |   |   |   |   |   |   |

## 9.5.2 Packet M/29/1

The coding used for X/28/1 is also used for packets M/29/1. This data applies to all Level 1 pages in magazine M but is overridden for a particular page if a packet X/28/1 exists for that page.
