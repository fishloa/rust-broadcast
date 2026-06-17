# Table 11: Region composition segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  region_composition_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  region_id | 8 | uimsbf  |
|  region_version_number | 4 | uimsbf  |
|  region_fill_flag | 1 | bslbf  |
|  reserved | 3 | bslbf  |
|  region_width | 16 | uimsbf  |
|  region_height | 16 | uimsbf  |
|  region_level_of_compatibility | 3 | bslbf  |
|  region_depth | 3 | bslbf  |
|  reserved | 2 | bslbf  |
|  CLUT_id | 8 | bslbf  |
|  region_8-bitpixel_code | 8 | bslbf  |
|  region_4-bitpixel-code | 4 | bslbf  |
|  region_2-bitpixel-code | 2 | bslbf  |
|  reserved | 2 | bslbf  |
|  while (processed_length < segment_length) { |  |   |
|  object_id | 16 | bslbf  |
|  object_type | 2 | bslbf  |
|  objectprovider_flag | 2 | bslbf  |
|  object_horizontal_position | 12 | uimsbf  |
|  reserved | 4 | bslbf  |
|  object_vertical_position | 12 | uimsbf  |
|  if (object_type ==0x01 or object_type == 0x02) { |  |   |
|  foregroundpixel_code | 8 | bslbf  |
|  backgroundpixel_code | 8 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |

## Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x11, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.



region_id: This 8-bit field uniquely identifies the region for which information is contained in this region_composition_segment.

region_version_number: This indicates the version of this region. The version number is incremented (modulo 16) if one or more of the following conditions is true:

the region_fill_flag is set;
the region's CLUT family has been modified;
the region has a non-zero length object list.

region_fill_flag: If set to '1', signals that the region is to be filled with the background colour defined in the region_n-bitpixel_code fields in this segment.

region_width: Specifies the horizontal length of this region, expressed in number of pixels. For subtitle services which do not include a display definition segment, the value in this field shall be within the range 1 to 720, and the sum of the region_width and the region_horizontal_address (see clause 7.2.1) shall not exceed 720. For subtitle services which include a display definition segment, the value of this field shall be within the range 1 to (display_width +1) and shall not exceed the value of (display_width +1) as signalled in the relevant DDS.

region_height: Specifies the vertical length of the region, expressed in number of pixels. For subtitle services which do not include a display definition segment, the value in this field shall be within the inclusive range 1 to 576, and the sum of the region_height and the region_vertical_address (see clause 7.2.1) shall not exceed 576. For subtitle services which include a display definition segment, the value of this field shall be within the range 1 to (display_height +1) and shall not exceed the value of (display_height +1) as signalled in the relevant DDS.

region_level_of_compatibility: This indicates the minimum type of CLUT that is necessary in the decoder to decode this region as defined in table 12.
