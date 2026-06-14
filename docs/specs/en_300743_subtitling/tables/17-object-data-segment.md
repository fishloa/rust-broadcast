# Table 17: Object data segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  object_data_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  object_id | 16 | bslbf  |
|  object_version_number | 4 | uimsbf  |
|  object_coding_method | 2 | bslbf  |
|  non_modifyingColour_flag | 1 | bslbf  |
|  reserved | 1 | bslbf  |
|  if (object_coding_method == '00'){ |  |   |
|  top_field_data_block_length | 16 | uimsbf  |
|  bottom_field_data_block_length | 16 | uimsbf  |
|  while(processed_length<top_field_data_block_length) |  |   |
|  pixel-data_sub-block() |  |   |
|  while (processed_length<bottom_field_data_block_length) |  |   |
|  pixel-data_sub-block() |  |   |
|  if (stuffing_length == 1) |  |   |
|  8_stuff_bits | 8 | bslbf  |
|  } |  |   |
|  if (object_coding_method == '01') { |  |   |
|  number of codes | 8 | uimsbf  |
|  for (i == 1, i <= number of codes, i ++) |  |   |
|  character_code | 16 | bslbf  |
|  } |  |   |
|  if (object_coding_method == '10'){ |  |   |
|  progressivepixel_block() |  |   |
|  } |  |   |
|  } |  |   |

# Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x13, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

object_id: Uniquely identifies within the page the object for which data is contained in this object_data_segment field.

object_version_number: Indicates the version of this segment data. When any of the contents of this segment change, this version number is incremented (modulo 16).

object_coding_method: Specifies the method used to code the object, as defined in table 18.
