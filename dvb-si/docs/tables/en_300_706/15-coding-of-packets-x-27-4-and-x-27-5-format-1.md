# Table 15: Coding of Packets X/27/4 and X/27/5, Format 1

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Triplet | Data Bits | Function  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
|  1-2 | 1-18 | Link 0  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 1-2 | Link Function These bits define the type of page being linked.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | Bit 2 | Bit 1 | Link Function |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 0 | 0 | Link to GPOP |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 0 | 1 | Link to POP |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 1 | 0 | Link to GDRCS |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 1 | 1 | Link to DRCS |   |   |   |   |   |   |   |   |   |   |   |   |
|  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 3-4 | Page Validity These bits define the presentation Levels requiring the linked page.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | Bit 4 | Bit 3 | Page Validity |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 0 | 0 | Reserved for future use |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 0 | 1 | Page required at Level 2.5 only |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 1 | 0 | Page required at Level 3.5 only |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | 1 | 1 | Page required at both Level 2.5 and 3.5 |   |   |   |   |   |   |   |   |   |   |   |   |
|  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 5-6 | Reserved for future use  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 7-10 | Page Number Units (LSB - MSB)  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 11 | Set to '1' (for compatibility with Format 2 packets X/27/4-7)  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 12-14 | Relative Magazine Number (LSB - MSB). These bits change the magazine number from that in byte 4 of this packet X/27. Setting any of these bits to '1' complements the corresponding magazine bit.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  1 | 15-18 | Page Number Tens (LSB - MSB)  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  2 | 1 | Set to '0' (for compatibility with Format 2 packets X/27/4-7)  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  2 | 2 | Reserved for future use  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  2 | 3-18 | Page Sub-code Flags These bits indicate the specific sub-pages required: '0' = Not required; '1' = Required.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|   |  |  | Data bits | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16  |
|   |  |  | S1 sub-code value | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13  |
|  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  3-4 | 1-18 | Link 1, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  5-6 | 1-18 | Link 2, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  7-8 | 1-18 | Link 3, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  9-10 | 1-18 | Link 4, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  11-12 | 1-18 | Link 5, coded the same as triplets 1 and 2.  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |
|  13 | 1-18 | Reserved for future use  |   |   |   |   |   |   |   |   |   |   |   |   |   |   |   |

When no particular page number is to be specified, the page number FF is transmitted and all the sub-code flags are set to '1'.



The function of first four links in a packet X/27/4 with the link coding of table 15 is fixed for Level 2.5, as shown in table 16. The Page Validity bits may also indicate their use at Level 3.5. The function of the remaining two links in a packet X/27/4 and the first two links in a packet X/27/5 is defined by the Link Function and Page Validity bits. These links do not contain information relevant to a Level 2.5 decoder.
