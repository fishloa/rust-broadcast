# Table 15: Object provider flag

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Object provision  |
| --- | --- |
|  0x00 | provided in the subtitling stream  |
|  0x01 | provided by a ROM in the IRD  |
|  0x02 | reserved  |
|  0x03 | reserved  |

object_horizontal_position: Specifies the horizontal position of the top left pixel of this object, expressed in number of horizontal pixels, relative to the left-hand edge of the associated region. The specified horizontal position shall be within the region, hence its value shall be in the range between 0 and (region_width -1).

object_vertical_position: Specifies the vertical position of the top left pixel of this object, expressed in number of lines, relative to the top of the associated region. The specified vertical position shall be within the region, hence its value shall be in the range between 0 and (region_height -1).

foregroundpixel_code: Specifies the entry in the applied 8-bit CLUT that has been selected as the foreground colour of the character(s).

backgroundpixel_code: Specifies the entry in the applied 8-bit CLUT that has been selected as the background colour of the character(s).

NOTE: IRDs with CLUT of four or sixteen entries find the foreground and background colours through the reduction schemes described in clause 9.

## 7.2.4 CLUT definition segment

The CLUT definition segment signals modifications to one or more CLUTs within a particular CLUT family. The modifications define replacement Recommendation ITU-R BT.601 [3] colours that can selectively modify one or more entries by replacing the default initial values (defined in clause 10). A subtitle service can thus create and use a CLUT consisting of a combination of colours in the default CLUT and colours not contained in the default CLUT. The segment syntax is defined in table 16.

For the purpose of backward compatibility of subtitle services with existing decoders, subtitle services shall support rendering in the Recommendation ITU-R BT.601 [3] colour space, via provision of the CDS, if not relying on the default CLUTs. This shall be the case even when the subtitle service makes use of the alternative_CLUT_segment (ACS) (defined in clause 7.2.8). However, in this case, for each ACS, a CDS with the same CLUT_id shall contain an entry for each of the colours used, using the 8-bits per entry option only, i.e. with the 8-bits per entry flag set to '1'. Each colour in the CDS shall be a colour within the Recommendation ITU-R BT.601 [3] colour space that is a close equivalent to the corresponding colour defined in the ACS.



The 8-bit CLUT entry format allows a sufficient number of colours to be used in order to achieve high quality anti-aliasing. This mitigates the effects of spatial upscaling, especially with UHDTV services. For the same reason, also when only the CDS is used with UHDTV services (i.e. no ACS is provided), it is recommended to use the 8-bit CLUT entry form of the CDS.
