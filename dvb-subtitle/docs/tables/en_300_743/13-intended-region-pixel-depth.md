# Table 13: Intended region pixel depth

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Intended region pixel depth  |
| --- | --- |
|  0x00 | reserved  |
|  0x01 | 2 bit  |
|  0x02 | 4 bit  |
|  0x03 | 8 bit  |
|  0x04..0x07 | reserved  |

CLUT_id: Identifies the family of CLUTs that applies to this region.

region_8-bitpixel-code: Specifies the entry of the applied 8-bit CLUT as background colour for the region when the region_fill_flag is set, but only if the region depth is 8 bit. The value of this field is undefined if a region depth of 2 or 4 bit applies.

region_4-bitpixel-code: Specifies the entry of the applied 4-bit CLUT as background colour for the region when the region_fill_flag is set, if the region depth is 4 bit, or if the region depth is 8 bit while the region_level_of_compatibility specifies that a 4-bit CLUT is within the minimum requirements. In any other case the value of this field is undefined.



region_2-bitpixel-code: Specifies the entry of the applied 2-bit CLUT as background colour for the region when the region_fill_flag is set, if the region depth is 2 bit, or if the region depth is 4 or 8 bit while the region_level_of_compatibility specifies that a 2-bit CLUT is within the minimum requirements. In any other case the value of this field is undefined.

processed_length: The total number of bytes that have already been processed following the segment_length field.

object_id: Identifies an object that is shown in the region.

object_type: Identifies the type of object as defined in table 14.
