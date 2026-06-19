# Table 21: Data type

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | data_type  |
| --- | --- |
|  0x10 | 2-bit/pixel code string  |
|  0x11 | 4-bit/pixel code string  |
|  0x12 | 8-bit/pixel code string  |
|  0x20 | 2_to_4-bit_map-table data  |
|  0x21 | 2_to_8-bit_map-table data  |
|  0x22 | 4_to_8-bit_map-table data  |
|  0xF0 | end of object line code  |
|  NOTE: All other values are reserved.  |   |


---


The data types 2-bit/pixel code string, 4-bit/pixel code string, and 8-bit/pixel code string are defined in clause 7.2.5.2.

A code '0xF0' = "end of object line code" shall be included after every series of code strings that together represent one line of the object.

2_to_4-bit_map-table: Specifies how to map the 2-bit/pixel codes on a 4-bit/entry CLUT by listing the 4 entry numbers of 4-bits each; entry number 0 first, entry number 3 last.
2_to_8-bit_map-table: Specifies how to map the 2-bit/pixel codes on an 8-bit/entry CLUT by listing the 4 entry numbers of 8-bits each; entry number 0 first, entry number 3 last.
4_to_8-bit_map-table: Specifies how to map the 4-bit/pixel codes on an 8-bit/entry CLUT by listing the 16 entry numbers of 8-bits each; entry number 0 first, entry number 15 last.
2_stuff_bits: Two stuffing bits that shall be coded as '00'.
4_stuff_bits: Four stuffing bits that shall be coded as '0000'.

bytealigned(): function is true if current position is aligned to whole byte boundary from the start of the pixel-data_sub-block().

## 7.2.5.2 Syntax and semantics of the pixel code strings

## 7.2.5.2.1 2-bits per pixel code

Table 22 defines the syntax of the 2-bits per pixel code string.
