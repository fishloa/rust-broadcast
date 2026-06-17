# Table 13: Coding of Packet M/29/4

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Bits | Function  |
| --- | --- | --- |
|  1 | 1-7 | Packet Function These bits define the application and scope of the data bits in the remainder of this packet.  |
|   |  | Bit  |
|   |  | 7 6 5 4 3 2 1  |
|   |  | 0 0 0 0 0 0 0  |
|   |  | The remaining data bits of this packet have an identical coding and function to those of packet X/28/4 (see clause 9.4.7) except that here the data applies to all pages in magazine M.  |
|   |  | All other values are reserved for future use.  |
|  1 | 8-14 | Default G0 and G2 Character Set Designation and National Option Selection As clause 9.4.2.2.  |
|  1 | 15-18 | Second G0 Set Designation and National Option Selection. As clause 9.4.2.2.  |
|  2 | 1-3 |   |
|  2 | 4 | Left Side Panel. As clause 9.4.2.2.  |
|  2 | 5 | Right Side Panel. As clause 9.4.2.2.  |
|  2 | 6 | Side Panel Status Flag. As clause 9.4.2.2.  |
|  2 | 7-10 | Number of Columns in Side Panels. As clause 9.4.2.2.  |
|  2 | 11-18 | Colour Map Entry Coding for CLUTs 0 and 1  |
|  3-12 | 1-18 | The bits are organized as 16 data words, each of 12 bits. Each word defines an entry in the  |
|  13 | 1-4 | Colour Map of clause 12.4, proceeding in transmission order from CLUT 0, entry 0 to CLUT 1, entry 7. Each 12 bit data word contains 4 bits for each primary colour (Red, Green and Blue), in the transmission order: RRRRGGGBBBB, with ascending order of bit significance within each 4 bits. CLUT 1, entry 0 is always "transparent". The corresponding bits for this entry should be ignored by decoders.  |
|  13 | 5-9 | Default Screen Colour. As clause 9.4.2.2.  |
|  13 | 10-14 | Default Row Colour. As clause 9.4.2.2.  |
|  13 | 15 | Black Background Colour Substitution. As clause 9.4.2.2.  |
|  13 | 16-18 | Colour Table Re-mapping for use with Spacing Attributes. As clause 9.4.2.2.  |

## 9.6 Packets for Page Linking

### 9.6.1 Packets X/27/0 to X/27/3 for Editorial Linking

Packets X/27 with designation codes in the range 0000 to 0011 define editorially linked pages. Codes of Practice exist for user-friendly page access methods. To support certain methods, a decoder is required to respond to the linked page data in packets X/27/0 and the display data in packets X/24 (see clause 11.1).

The structure of packets X/27/0 - 3 is shown in figure 12.


---


![img-0.jpeg](img-0.jpeg)
Figure 12: Format of packets X/27/0-3 for editorial links

Byte 6 is the designation code, coded Hamming 8/4. Bytes 7 to 42 are also coded Hamming 8/4 and are arranged as 6 groups of 6 bytes. Each group of 6 bytes defines a linked page address, the groups being numbered 0 to 5 in order of transmission. Bytes 43 to 45 are defined for packets X/27/0 only (see table 14).

Each linked page address has the same format as bytes 6 to 11 of a page header packet (see clause 9.3.1) and contains:

Relative magazine number: 3 bits;

Page number: 8 bits;

Page sub-code: 13 bits.

The bits M1, M2, M3 shown in figure 12 correspond to the control bits C4, C5 and C6 in the page header packet. They are used here to change the magazine number from that in byte 4 of this packet X/27. Setting any of these bits to '1' complements the corresponding magazine bit.

When no particular page number is to be specified, the page number FF is transmitted. When no particular page subcode is to be specified, the page sub-code 3F7F is transmitted. When the page address XFF:3F7F is transmitted, no page is specified.

The mapping of the linked page addresses to the bytes of the packet is shown in table 14.
