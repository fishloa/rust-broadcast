# Table 28: End of display set segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  end_of_display_set_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  } |  |   |

# Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x80, as listed in table 7.

page_id: If the subtitle service uses shared data, then the page_id shall be coded with the ancillary page id value signalled in the subtitling descriptor. Otherwise the page_id shall have the value of the composition page id.

segment_length: This field shall be set to the value zero.

# 7.2.7 Disparity Signalling Segment

The Disparity Signalling Segment (DSS) supports the subtitling of plano-stereoscopic 3DTV content by allowing disparity values to be ascribed to a region or to part of a region. Whilst regions cannot themselves share scan lines the DSS defines subregions which may be assigned different individual disparity values.

Absence of a DSS implies that the stream has been coded in accordance with ETSI EN 300 743 (V1.3.1) [6] to provide subtitles intended for 2D presentation. In such cases decoders capable of supporting 3D services shall apply an implicit disparity of zero.

Each region can contain one or more subregions referenced to that region. Subregions have the same height as their region and may not overlap horizontally (see figures 5 and 6).

There shall be no more than 4 subregions per region and no more than 4 subregions per display set.

A subregion shall enclose all the objects for which it conveys a particular disparity value and all objects shall be enclosed by one of the subregions of a region. All active subregions in a declared display set shall be signalled in the DSS.

A change to any data (e.g. disparity values) signalled in the DSS requires a change to the DSS version number but does not require a change to the version number of the RCSs nor the retransmission of the RCS if the relevant region definition itself remains unchanged.



Disparity is the difference between the horizontal positions of a pixel representing the same point in space in the right and left views of a plano-stereoscopic image. Positive disparity values move the subtitle objects enclosed by a subregion away from the viewer whilst negative values move them towards the viewer. A value of zero places the objects enclosed by that subregion in the plane of the display screen.

To ensure that subtitles are placed at the correct depth and horizontal location the disparity shift values signalled shall be applied symmetrically to each view of any subregion and by implication any object bounded by the subregion. A positive disparity shift value for example of +7 will result in a shift of 7 pixels to the left in the left subtitle subregion image and a shift of 7 pixels to the right in the right subtitle subregion image. A negative disparity shift value of -7 will result in a shift of 7 pixels to the right in the left subtitle subregion image and a shift of 7 pixels to the left in the right subtitle subregion image. Note that the actual disparity of the displayed subtitle is therefore double the value of the disparity shift values signalled in the disparity integer and/or fractional fields carried in the DSS.

Encoders shall assign a value of disparity to the default disparity (and its associated disparity_update_sequence if present) which would result in an appropriate placement of the subtitles were a decoder only able to apply the default disparity to the entire display set at that time. Decoders which can support only one value of disparity per page shall apply the default disparity value to each region.

Decoders which can attribute a separate disparity value to each region (or subregion) shall parse the region loop in the DSS syntax and implement the signalled disparity shift values for the declared regions or subregions.

Encoders shall ensure that the relative position and size of multiple subregions are managed so as to avoid horizontal overlap when the objects enclosed within those subregions have the relevant disparity values applied as a shift by the decoder. In the event, however, that a decoder is presented with subregions whose views do overlap, the decoder should manage occlusion appropriately (for example by presenting those subregions in depth-order of perceived proximity to the viewer i.e. the foremost shown in its entirety).

Encoders that are generating streams which include a DSS shall encode the background of a region using the region fill mechanism only if the region contains a single subregion or if the region fill indexes a fully transparent CLUT entry.

A stream with a DSS shall include a Display Definition Segment and the display window parameters of that DDS shall be consistent with the application of the disparity values signalled in the DSS.

In the transmission of a display set (new or updated) the DSS will normally follow the RCS. However, if the PCS has page_state = normal and if the only changes to be signalled are disparity values, these values may be updated by the simple transmission of a DDS, a DSS and an EDS.

![img-0.jpeg](img-0.jpeg)
Figure 5: Different subtitles sharing a region



![img-1.jpeg](img-1.jpeg)
Figure 6: Different subtitles assigned to different subregions within one region

Temporal updates to disparity values may be encoded by different strategies. One simple method is to transmit successive DSSs whose signalled values are timed to the PTS of their respective PES packets. Another potentially more bit-rate efficient method uses the DSS to signal a succession of disparity updates using the disparity_shift_update_sequence mechanism defined below. Note that a mixed approach is also possible in which, for example, a DSS which includes a disparity_shift_update_sequence is followed (and possibly overruled) by a DSS with a new disparity_shift_update_sequence or by a DSS which signals a new set of disparity values timed to the PTS.

The disparity shift update sequence mechanism is illustrated in figure 7 and in annex C. A succession of near-future disparity values are transmitted together, defined at intervals which can vary, and are applied at times which can easily be calculated from the PTS and the transmitted interval parameters. Intermediate disparity values may be interpolated by the decoder as appropriate within the capabilities of the decoder (two possible interpolation approaches are indicated in figure 7 by hatched lines). Care should be taken in interpolation to avoid "overshoot" in the calculated intermediate disparity values (particularly for positive values).



![img-2.jpeg](img-2.jpeg)
Figure 7: Disparity updates using the disparity_shift_update_sequence mechanism

Experiments have shown that some legacy 2D IRDs do not behave in a predictable and user-friendly manner when presented with subtitle streams which contain a DSS.

Broadcasters, service providers and network operators should note that services intended for 2D IRDs but derived from 3D services should therefore include subtitle streams coded in accordance with ETSI EN 300 743 (V1.3.1) [6] i.e. without a DSS. In the case of service-compatible 3D this may involve providing two subtitle streams per language carried on separate PIDs (with and without a DSS) and distinguishing the 2D and 3D versions of the service appropriately in the PSI.

The syntax of the disparity signalling segment is shown in table 29.
