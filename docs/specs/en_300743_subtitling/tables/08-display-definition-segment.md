# Table 8: Display definition segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  display_definition_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | uimsbf  |
|  segment_length | 16 | uimsbf  |
|  dds_version_number | 4 | uimsbf  |
|  display_window_flag | 1 | uimsbf  |
|  reserved | 3 | uimsbf  |
|  display_width | 16 | uimsbf  |
|  display_height | 16 | uimsbf  |
|  if (display_window_flag == 1) { |  |   |
|  display_window_horizontal_position_minimum | 16 | uimsbf  |
|  display_window_horizontal_position_maximum | 16 | uimsbf  |
|  display_window_vertical_position_minimum | 16 | uimsbf  |
|  display_window_vertical_position_maximum | 16 | uimsbf  |
|  } |  |   |
|  } |  |   |

Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x14, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

dds_version_number: The version of this display definition segment. When any of the contents of this display definition segment change, this version number is incremented (modulo 16).

display_window_flag: If display_window_flag = 1, the DVB subtitle display set associated with this display definition segment is intended to be rendered in a window within the display resolution defined by display_width and display_height. The size and position of this window within the display is defined by the parameters signalled in this display definition segment as display_window_horizontal_position_minimum, display_window_horizontal_position_maximum, display_window_vertical_position_minimum and display_window_vertical_position_maximum.

If display_window_flag = 0, the DVB subtitle display set associated with this display_definition_segment is intended to be rendered directly within the display resolution defined by display_width and display_height.

display_width: Specifies the maximum horizontal width of the display in pixels minus 1 assumed by the subtitling stream associated with this display definition segment. The value in this field shall be in the region 0..4095.

display_height: Specifies the maximum vertical height of the display in lines minus 1 assumed by the subtitling stream associated with this display definition segment. The value in this field shall be in the region 0..4095.

display_window_horizontal_position_minimum: Specifies the left-hand most pixel of this DVB subtitle display set with reference to the left-hand most pixel of the display.

display_window_horizontal_position_maximum: Specifies the right-hand most pixel of this DVB subtitle display set with reference to the left-hand most pixel of the display.

display_window_vertical_position_minimum: Specifies the upper most line of this DVB subtitle display set with reference to the top line of the display.

display_window_vertical_position_maximum: Specifies the bottom line of this DVB subtitle display set with reference to the top line of the display.



![img-0.jpeg](img-0.jpeg)
Figure 4: Use of Display definition segment parameters

HDTV and UHDTV IRDs that offer a means of scaling or positioning the subtitles under user control (e.g. to make them larger or smaller) can use the information conveyed in the display definition segment to determine safe strategies for zooming and/or positioning that will ensure that windowed subtitles can remain visible. However, scaling operations are not recommended for subtitles that have been anti-aliased for their original graphical resolution. Any scaling applied to such subtitles could degrade them significantly and thereby impact their readability.

# 7.2.2 Page composition segment

The page composition for a subtitle service is carried in page_composition_segments. The page_id of each page_composition_segment shall be equal to the composition_page_id value provided by the subtitling descriptor.

The syntax of the page_composition_segment is shown in table 9.
