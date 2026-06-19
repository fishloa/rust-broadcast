# Table 30: disparity_shift_update_sequence

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  disparity_shift_update_sequence() { |  |   |
|  disparity_shift_update_sequence_length | 8 | bslbf  |
|  interval_duration[23..0] | 24 | uimsbf  |
|  division_period_count | 8 | uimsbf  |
|  for (i= 0; i< division_period_count; i++) { |  |   |
|  interval_count | 8 | uimsbf  |
|  disparity_shift_update_integer_part | 8 | tcimsbf  |
|  } |  |   |
|  } |  |   |

Semantics:

processed_length: The total number of bytes that have already been processed following the segment_length field.

region_id: Identifies the region to which the following subregion data refers. Regions which have been declared in the display set but which are not referenced in the while-loop has to adopt the page_default_disparity and its associated disparity_update_sequence where present.

disparity_shift_update_sequence_region_flag: If '1' then a disparity_shift_update_sequence is included for all subregions of this region. If '0' then a disparity_shift_update_sequence for this region is not included.

number_of_subregions_minus_1: The number of subregions minus one which apply to this region. If number_of_subregions_minus_1 = 0 then the region has only one subregion whose dimensions are the same as the region and the signalled disparity therefore applies to the whole region.

subregion_horizontal_position: Specifies the left-hand most pixel position of this subregion. This value shall always fall within the declared extent of the region of which this is a subregion and shall therefore be in the range 0..4095. Note that as with the region positional specification this horizontal position is relative to the page.

subregion_width: Specifies the horizontal width of this subregion expressed in pixels. The combination of subregion_horizontal_position and subregion_width shall always fall within the declared extent of the region to which this refers. The value of this field shall therefore be in the range 0..4095.

subregion_disparity_shift_integer_part: Specifies the integer part of the disparity shift value which should be applied to all subtitle pixel data enclosed within this subregion. This allows the disparity to range between +127 and -128 pixels.

subregion_disparity_shift_fractional_part: Specifies the fractional part of the disparity shift value which should be applied to all subtitle pixel data enclosed within this subregion. When used as an extension of the integer part, this allows the signalled disparity shift to be defined to  $1/16$  pixel accuracy. Note that this fractional part is unsigned (0b0001 represents  $1/16$  pixel and 0b1111 represents  $15/16$  pixel) and should be combined with the integer part always by adding the fractional part to the integer part. A disparity value of -0,75 is therefore signalled as [-1, 0,25] and a value of -4,5 as [-5, 0,5].

NOTE 2: Any processing (either at the encoder or the decoder) which needs to implement only integer values of disparity shift has to ensure values are rounded "towards the viewer" (i.e. that positive values of disparity are rounded down and negative values rounded up).

disparity_shift_update_sequence_length: Specifies the number of bytes contained in the disparity_shift_update_sequence which follows this field.

interval_duration: Specifies the unit of interval used to calculate the PTS for the disparity update as a 24-bit field (in  $90\mathrm{kHz}$  STC increments). The value of interval_duration shall correspond to an exact multiple  $(\geq 1)$  of frame periods and its maximum value is therefore just over 186 seconds.

division_period_count: Specifies the number of unique disparity values  $(\geq 1)$  and hence the number of time intervals within the following disparity_shift_update_sequence 'for' loop.

interval_count: Specifies the multiplier used to calculate the PTS for this disparity update from the initial PTS value. The calculation for the PTS for this update is  $\mathrm{PTS}_{\mathrm{new}} = \mathrm{PTS}_{\mathrm{previous}} + (\mathrm{interval\_duration} \times \mathrm{interval\_count})$  where interval count  $\geq 1$ , where  $\mathrm{PTS}_{\mathrm{new}}$  increases with every iteration of the loop and where the initial value of  $\mathrm{PTS}_{\mathrm{previous}}$  is the PTS signalled in the PES header.



disparity_shift_update_integer_part: Specifies the integer part of the disparity update value which should be applied to all subtitle pixel data enclosed within this page or this subregion. This allows the disparity to excurse +127 to -128 pixels.

# 7.2.8 Alternative CLUT segment

The versions of the present document prior to V1.6.1 defined CLUTs exclusively in Recommendation ITU-R BT.601 [3] colour space. The alternative_CLUT_segment (ACS) permits a CLUT to be defined in other colour systems. The syntax of the ACS is shown in table 31.

For the purpose of optimal backwards compatibility of subtitle services and existing decoders, when a subtitle service makes use of the alternative_CLUT_segment (ACS), it shall also provide the legacy capability of rendering in the ITU-R BT.601 [3] colour space, by the provision of a CDS within the same CLUT family (with the same CLUT_id) that contains the same number of entries as the ACS, so that IRDs that do not support the ACS can perform their own conversion from the Recommendation ITU-R BT.601 [3] colours for the rendering of the subtitles with non-Recommendation ITU-R BT.601 [3] video content.

The ACS permits a CLUT with up to 256 colours. This allows a sufficient number of colours to be used in order to achieve high quality anti-aliasing. This mitigates the effects of spatial upscaling, especially with UHDTV services.
