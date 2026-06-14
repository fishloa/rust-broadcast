# Table 2: Control bits in the page header

_Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing)._


|  Control Bit | Location | Function  |
| --- | --- | --- |
|  C4 Erase Page | Byte 9, bit 8 | Packets X/1 to X/28 belonging to a previous transmission of the page should be erased from the decoder's memory before packets belonging to the associated page are stored.  |
|  C5 Newsflash | Byte 11, bit 6 | When set to '1' this bit indicates that the associated page is a Newsflash page. All information intended for display on such a page will be boxed and will be displayed inset into the normal video picture.  |
|  C6 Subtitle | Byte 11, bit 8 | When set to '1' this bit indicates that the associated page is a subtitle page. All information intended for display on such a page will be boxed and will be displayed inset into the normal video picture.  |
|  C7 Suppress Header | Byte 12, bit 2 | Data addressed to row 0 is not to be displayed.  |
|  C8 Update Indicator | Byte 12, bit 4 | Data within packets X/1 to X/28 of the associated page has been changed since the previous transmission. The setting of this bit is under editorial control.  |
|  C9 Interrupted Sequence | Byte 12, bit 6 | The associated page is not in numerical order of page sequence, allowing the header to be excluded from a rolling header display to avoid discontinuities.  |
|  C10 Inhibit Display | Byte 12, bit 8 | Data addressed to rows 1 to 24 is not to be displayed.  |
|  C11 Magazine Serial | Byte 13, bit 2 | When set to '1' the service is designated to be in Serial mode and the transmission of a page is terminated by the next page header with a different page number. When set to '0' the service is designated to be in Parallel mode and the transmission of a page is terminated by the next page header with a different page number but the same magazine number. The same setting shall be used for all page headers in the service.  |
|  C12, C13, C14 National Option Character Subset | Byte 13, bits 4, 6 and 8 | Where the decoder is capable of displaying text in more than one language these control bits are used to select G0 character set options, (see clause 15.2). The response to these control bits may be modified by packets X/28/0 Format 1, X/28/4, M/29/0 and M/29/4.  |



## 9.3.1.4 Data bytes

Bytes 14 to 45 in page header packets carry 32 character or display control codes, coded 7 data bits plus one bit odd parity. They are normally intended for display. Bytes 38 to 45 are usually coded to represent a real-time clock.

## 9.3.2 Packets X/1 to X/25

Packets X/1 to X/25 intended for direct display are coded according to figure 10.

![img-1.jpeg](img-1.jpeg)
Figure 10: Format of packets X/1 to X/25 for direct display

The same coding is used for the packets X/1 to X/24 of DRCS data pages. Different coding schemes are used for packets X/1 to X/25 when they form part of pages not intended for direct display such as Object definition pages (see clause 10.5.1), magazine inventory pages (see clause 11.3), the additional data pages used in the "TOP" system (see clause 11.2), and for data broadcasting, EN 300 708 [2].

## 9.4 Page enhancement data packets

Packets X/26, X/28 and M/29 can carry data to enhance a basic Level 1 Teletext page. The general coding scheme is shown in figure 11. Byte 6 is used as an additional address byte (designation code), coded Hamming 8/4. This allows up to 16 versions of each packet type. The remaining 39 bytes are Hamming 24/18 coded, grouped as 13 triplets.

![img-2.jpeg](img-2.jpeg)
Figure 11: Format of packets X/26, X/28 and M/29

NOTE: Packets X/1 to X/25 of POPs and GPOPs use the same coding scheme for bytes 7 to 45. Byte 6 is Hamming 8/4 coded but does not have the function of a designation code (see clauses 10.5.1.2 and 10.5.1.3).



## 9.4.1 Packet X/26

Packets X/26 are used for:

- at presentation Levels 1.5, 2.5, 3.5: addressing a character location and overwriting the existing character defined on the Level 1 page;
- at presentation Levels 2.5, 3.5: modifying existing display attributes and for object definitions;
- at all presentation Levels: VCR programming, see EN 300 231 [1].

Designation code values 0000 to 1111 allow up to 16 packets with Y = 26 to be associated with a given page.

Unlike other page enhancement packets, the function of a data bit within a packet X/26 is not determined by its overall position within the packet. The coding and function of the data bits of packets X/26 is described in clause 12.3.

## 9.4.2 Packet X/28/0 Format 1

### 9.4.2.1 Page Function and Page Coding

A Format 1 packet X/28 with a designation code value of 0000 may be transmitted as part of any page at any presentation level. The first 7 data bits of the packet define the function and the coding of packets X/1 to X/25 of the associated page, as shown in table 3. This coding scheme is also used for the first 7 data bits of packets X/28/3 and X/28/4.
