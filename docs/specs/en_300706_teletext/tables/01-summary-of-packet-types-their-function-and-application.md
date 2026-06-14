# Table 1: Summary of packet types, their function and application

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Packet | Function and application | Presentation Level 1 1.5 2.5 3.5  |
| --- | --- | --- |
|  X/0 (Page header) | Acts as both a page identifier and a page terminating packet. Decoders should respond to packets X/0 for ALL possible page numbers and sub-codes, including those with hexadecimal elements in their address. Any packet X/0 may be used for both time filling and page terminating applications. NOTE: It is not intended that the viewer should be provided with the means to select directly pages with hexadecimal elements in their address. | ☐ ☐ ☐ ☐  |
|  X/1 to X/23 (see note 1) | These packets carry the display data of basic Teletext pages, coded 7 data bits plus 1 odd parity bit. Other forms of coding may be used when the page does not carry data intended for direct display. | ☐ ☐ ☐ ☐  |
|   | Used for navigational purposes in the TOP Code of Practice, (see clause 11.2). | ☐ ☐ ☐ ☐  |
|   | Used for pages carrying enhancement data not intended for direct display, e.g. objects definitions and DRCS data. | ☐ ☐ ☐ ☐  |
|  X/24 (see note 1) | Used for navigational purposes in the FLOF Code of Practice, (see clause 11.1). | ☐ ☐ ☐ ☐  |
|   | Used for pages carrying enhancement data not intended for direct display, e.g. objects definitions and DRCS data. | ☐ ☐ ☐ ☐  |
|  X/25 (see note 1) | As part of a basic Teletext display page, the packet carries a number of displayable labels relating to the data in the page for key-word search applications. | ☐ ☐ ☐ ☐  |
|   | Used for pages carrying enhancement data not intended for direct display, e.g. objects definitions. | ☐ ☐ ☐ ☐  |
|  X/26/0 - 14 (see note 1) | Used to carry codes for programming ancillary equipment such as video recorders, EN 300 231 [1]. | ☐ ☐ ☐ ☐  |
|   | Used to address character locations within a page and define new characters to be written to these locations. This has the action of overwriting the character defined for this location on the Level 1 page. A Level 1.5 decoder may respond to some or all of the column address group triplets (see clause 12.3.4) which access the G0, G2 and G3 character sets. | ☐ ☐ ☐ ☐  |
|  X/26/0 - 15 (see note 1) | Used to address character locations within a page including any side-panels. They can select and place alphanumeric and mosaics characters from the G0, G1, G2 and G3 sets, redefinable characters, non-spacing attributes and objects. | ☐ ☐ ☐ ☐  |
|   | Used for object definition pages. | ☐ ☐ ☐ ☐  |
|  X/27/0 | Used for editorial page linking. An example of their use is the FLOF Code of Practice, (see clause 11.1). | ☐ ☐ ☐ ☐  |
|  X/27/1 - 3 | Provide additional links to editorial pages. | (see note 2)  |
|  X/27/4 Format 1 | Used for compositional page linking to objection definition and DRCS pages. | ☐ ☐ ☐ ☐  |
|  X/27/5 Format 1 | Used for compositional page linking to objection definition and DRCS pages. | ☐ ☐ ☐ ☐  |
|  X/27/6 - 7 Format 1 | Provide additional compositional links. | (see note 2)  |
|  X/27/4 - 7 Format 2 | Used for compositional page linking in data broadcasting applications | ☐ ☐ ☐ ☐  |
|  X/27/8 - 15 | Use not currently defined. |   |
|  X/28/0 Format 1 | Page specific data: Page function Page coding | ☐ ☐ ☐ ☐ (see note 3)  |
|  X/28/0 Format 1 | Page specific data (presentation related): Character set designation Size and position of side-panels Colour Map (CLUTs 2 and 3) Default screen colour Default row colour Black background substitution by row colour Colour table re-mapping of the foreground and background colours of the Level 1 page. | ☐ ☐ ☐ ☐  |
|  X/28/0 Format 2 | Page specific data for Page Format - CA type data broadcasting pages defined according to EN 300 708 [2] clause 5. | ☐ ☐ ☐ ☐  |



|  Packet | Function and application | Presentation Level 1 1.5 2.5 3.5  |
| --- | --- | --- |
|  X/28/1 | Page specific data (presentation related): Character set designation (according to earlier specifications, note 4) | ○ ⊕ ○ ○  |
|  X/28/1 | Page specific data (presentation related): DCLUT4 for global 12x10x2 DRCS mode characters DCLUT4 for normal 12x10x2 DRCS mode characters DCLUT16 for global 12x10x4 and 6x5x4 DRCS modes characters DCLUT16 for normal 12x10x4 and 6x5x4 DRCS modes characters. | ○ ○ ○ ⊕  |
|  X/28/2 | Contains a Page Key for the descrambling of the encrypted data contained in packets X/1 - X/25 of the associated data broadcasting page. See EN 300 708 [2]. | ○ ○ ○ ○  |
|  X/28/3 | Page specific data (related to DRCS downloading pages): Page function Page coding DRCS downloading mode invocation. | ○ ○ ⊕ ⊕  |
|  X/28/4 | Page specific data (presentation related): Page function Page coding Character set designation Size and position of side-panels Colour Map (CLUTs 0 and 1) Default screen colour Default row colour Black background substitution by row colour Colour table re-mapping of the foreground and background colours of the Level 1 page. | ○ ○ ○ ⊕  |
|  X/28/5 - 15 | Use not currently defined. |   |
|  M/29/0 | Same functions (apart from page function and coding) as defined for packets X/28/0 Format 1 except that the information applies to all pages in magazine M unless overridden for a particular page by a packet X/28/0 Format 1. | ○ ○ ⊕ ⊕  |
|  M/29/1 | Character set designation (according to earlier specifications). Applies to all pages in magazine M unless overridden for a particular page by a packet X/28/1. | ○ ⊕ ○ ○ (see note 4)  |
|  M/29/2 - 3 | Use not currently defined. |   |
|  M/29/4 | Same functions (apart from page function and coding) as defined for packets X/28/4 except that the information applies to all pages in magazine M unless overridden for a particular page by a packet X/28/4. | ○ ○ ○ ⊕  |
|  M/29/5 - 15 | Use not currently defined. |   |
|  1 - 3/30 5 - 7/30 | Use not currently defined, though in some countries these packets may be in use for independent data services. |   |
|  4/30 | Proposed use: Audio description data for the visually impaired. |   |
|  8/30/0 - 1 | Broadcast service data packet, Format 1. Includes multiplexed operation flag, the page number of a suitable initial page, the current time and date, network identification codes, and a text message. | ⊕ ⊕ ⊕ ⊕  |
|  8/30/2 - 3 | Broadcast service data packet, Format 2. Includes multiplexed operation flag, the page number of a suitable initial page, programme identification codes and control data for video recorders, and a text message. | ⊕ ⊕ ⊕ ⊕  |
|  8/30/4 - 15 | Use not currently defined. |   |
|  8/31 - 3/31 | Independent data services. | ⊕ ⊕ ⊕ ⊕  |
|  4/31 - 7/31 | Use not currently defined. |   |
|  NOTE 1: Where a packet has more than one entry in this table, the precise function and coding of a given packet is determined from the type of page to which it belongs. This may be ascertained from a packet X/28/0 Format 1, if transmitted, or by the context in which the page was referenced, e.g. a MOT entry pointing to an object definition page, or by a Code of Practice, e.g. TOP. NOTE 2: Application not currently defined. NOTE 3: Can form part of any page at any presentation Level to define its function and coding but its transmission is not mandatory. NOTE 4: Function superseded by the present document.  |   |   |

## 9.2 Reserved bits

Decoders should ignore bits and bytes which are indicated as being reserved for future use.



# 9.3 Directly displayable data packets

# 9.3.1 Page header

Page header packets  $(\mathrm{Y} = 0)$  comprises three main elements: page address, control bits and data normally intended for display, as shown in figure 9. The page address consists of a page number and a page sub-code.

![img-0.jpeg](img-0.jpeg)
Figure 9: Format of the page header packet (X/0)

# 9.3.1.1 Page number

The page number is defined by bytes 6 and 7, both Hamming  $8/4$  protected. The page number comprises page units and page tens elements:

|  Function | Byte | Data Bit | Weighting | Range  |
| --- | --- | --- | --- | --- |
|  Page Units | 6 | 2 | 20 | 0 - F  |
|   |   |  4 | 21  |   |
|   |   |  6 | 22  |   |
|   |   |  8 | 23  |   |
|  Page Tens | 7 | 2 | 20 | 0 - F  |
|   |   |  4 | 21  |   |
|   |   |  6 | 22  |   |
|   |   |  8 | 23  |   |

NOTE: Odd numbered data bits carry the Hamming 8/4 protection bits.



## 9.3.1.2 Page sub-code

The page sub-code is defined by byte 8, part of byte 9, byte 10 and part of byte 11, all Hamming 8/4 protected. The page sub-code comprises four elements S1, S2, S3 and S4:

|  Function | Byte | Data Bit | Weighting | Range  |
| --- | --- | --- | --- | --- |
|  S1 (least significant) | 8 | 2 | 2^{0} | 0 - F  |
|   |   |  4 | 2^{1}  |   |
|   |   |  6 | 2^{2}  |   |
|   |   |  8 | 2^{3}  |   |
|  S2 | 9 | 2 | 2^{0} | 0 - 7  |
|   |   |  4 | 2^{1}  |   |
|   |   |  6 | 2^{2}  |   |
|  S3 | 10 | 2 | 2^{0} | 0 - F  |
|   |   |  4 | 2^{1}  |   |
|   |   |  6 | 2^{2}  |   |
|   |   |  8 | 2^{3}  |   |
|  S4 (most significant) | 11 | 2 | 2^{0} | 0 - 3  |
|   |   |  4 | 2^{1}  |   |

NOTE: Odd numbered data bits carry the Hamming 8/4 protection bits.

## 9.3.1.3 Control bits

The page control bits, C4 to C14, are described in table 2. They are transmitted in bytes 9, 11, 12 and 13 of the page header packet and are all Hamming 8/4 protected. The control bits are active on being set to '1'.
