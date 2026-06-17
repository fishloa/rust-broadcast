# Table 12: Region level of compatibility

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Minimum CLUT type  |
| --- | --- |
|  0x00 | reserved  |
|  0x01 | 2-bit/entry CLUT required  |
|  0x02 | 4-bit/entry CLUT required  |
|  0x03 | 8-bit/entry CLUT required  |
|  0x04..0x07 | reserved  |

If the decoder does not support the specified minimum requirement for the type of CLUT, then this region shall not be displayed, even though some other regions, requiring a lesser type of CLUT, may be presented.

region_depth: Identifies the intended pixel depth for this region as defined in table 13.
