# Table 29: Disparity signalling segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  disparity_signalling_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  dss_version_number | 4 | uimsbf  |
|  disparity_shift_update_sequence_page_flag | 1 | bslbf  |
|  reserved | 3 | bslbf  |
|  page_default_disparity_shift | 8 | tcimsbf  |
|  if (disparity_shift_update_sequence_page_flag ==1) { |  |   |
|  disparity_shift_update_sequence() |  |   |
|  } |  |   |
|  while (processed_length  |   |   |
|  region_id | 8 | uimsbf  |
|  disparity_shift_update_sequence_region_flag | 1 | bslbf  |
|  reserved | 5 | uimsbf  |
|  number_of_subregions_minus_1 | 2 | uimsbf  |
|  for (n=0; n<= number_of_subregions_minus_1; n++) { |  |   |
|  if (number_of_subregions_minus_1 > 0) { |  |   |
|  subregion_horizontal_position | 16 | uimsbf  |
|  subregion_width | 16 | uimsbf  |
|  } |  |   |
|  subregion_disparity_shift_integer_part | 8 | tcimsbf  |
|  subregion_disparity_shift_fractional_part | 4 | uimsbf  |
|  reserved | 4 | uimsbf  |
|  if (disparity_shift_update_sequence_region_flag ==1) { |  |   |
|  disparity_shift_update_sequence() |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |

Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x15, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

dss_version_number: Indicates the version of this DSS. The version number is incremented (modulo 16) if any of the parameters for this particular DSS are modified.

disparity_shift_update_sequence_page_flag: If '1' then the disparity_shift_update_sequence immediately following is to be applied to the page_default_disparity_shift. If '0' then a disparity_shift_update_sequence for page_default_disparity_shift is not included.

page_default_disparity_shift: Specifies the default disparity value which should be applied to all regions within the page (and thus to all objects within those regions) in the event that the decoder cannot apply individual disparity values to each region. This disparity value is a signed integer and thus allows the default disparity to range between +127 and -128 pixels.

NOTE 1: Any decoder which can apply separate disparity values to a region or subregion has to apply the relevant values to any subregions signalled in the region loop.

disparity_shift_update_sequence: The syntax of this field is specified in table 30.
