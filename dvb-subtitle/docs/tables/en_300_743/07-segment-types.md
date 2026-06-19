# Table 7: Segment types

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Segment type | Cross-reference  |
| --- | --- | --- |
|  0x10 | page composition segment | defined in clause 7.2.2  |
|  0x11 | region composition segment | defined in clause 7.2.3  |
|  0x12 | CLUT definition segment | defined in clause 7.2.4  |
|  0x13 | object data segment | defined in clause 7.2.5  |
|  0x14 | display definition segment | defined in clause 7.2.1  |
|  0x15 | disparity signalling segment | defined in clause 7.2.7  |
|  0x16 | alternative_CLUT_segment | defined in clause 7.2.8  |
|  0x17 - 0x7F | reserved for future use |   |
|  0x80 | end of display set segment | defined in clause 7.2.6  |
|  0x81 - 0xEF | private data |   |
|  0xFF | stuffing (see note) |   |
|  All other values | reserved for future use |   |
|  NOTE: The present document does not define a syntax for stuffing within the PES. In applications where stuffing is deemed to be necessary (for example for monitoring or for network management reasons) implementers of DVB subtitle coding equipment are strongly advised to use the transport packet adaptation field for stuffing since that method will usually place no processing overhead on the subtitle encoder.  |   |   |

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: The segment_length shall specify the number of bytes contained in the immediately following segment_data_field.

segment_data_field: This is the payload of the segment. The syntax of this payload depends on the segment type, and is defined in clauses 7.2.1 to 7.2.8.



## 7.2.0.2 Forward compatibility

The segment structure allows forward compatibility with future revisions of the present document.

NOTE: IRDs are expected to be robust against new segment types that might be added in future revisions of the present document. IRDs are also expected to be robust against the backward compatible addition or extension of data structures, and the assignment of reserved element values in future revisions of the present document.

The following explicit requirement for IRD forward compatibility was added in version 1.6.1 of the present document. Thus its mandatory nature is limited to IRDs with "UHDTV" subtitling support as defined in table 35 (in clause 7.3, interoperability points). For all other IRDs, forward compatibility is recommended.

IRDs shall ignore segment types that they do not support, without impacting decoding of segment types they do support. If the IRD encounters unknown structures or reserved values within a segment, then it shall decode the parts it is able to decode, or ignore the segment.

## 7.2.1 Display definition segment

The display definition for a subtitle service may be defined by the display definition segment (DDS).

Absence of a DDS in the subtitle service implies that the stream is coded in accordance with ETSI EN 300 743 (V1.2.1) [5] and that a display resolution of 720 by 576 pixels may be assumed, i.e. the subtitle service is associated with an SDTV service. Such streams will nevertheless be decodable by subtitling decoders that are compliant with any later versions of the present document. Subtitle streams associated with HDTV services may include the DDS.

Subtitle streams associated with UHDTV services shall include the DDS, whereby subtitle graphics rendering shall be constrained to HDTV resolution. If no display window is signalled, then the IRD shall apply a resolution upscale of factor two in both horizontal and vertical directions when rendering the subtitles on a UHDTV resolution screen. If the display window feature is used with subtitles for a UHDTV service, then the display window shall be specified as having dimension no larger than the maximum display resolution for HDTV, i.e. 1920 by 1080 pixels, within the larger UHDTV display resolution, which may be any of the dimensions allowed in ETSI TS 101 154 [9], up to the maximum display resolution for UHDTV, i.e. 3840 by 2160 pixels. Hence with UHDTV the display_window_horizontal_position_maximum minus display_window_horizontal_position_minimum shall be no more than 1919, and the display_window_vertical_position_maximum minus display_window_vertical_position_minimum shall be no more than 1079. When the display window feature is used then the IRD shall not upscale the subtitle object spatially. As specified in clause 6.3, subtitle streams that are intended to be decoded by decoders that are compliant with ETSI EN 300 743 (V1.2.1) [5] shall not include a DDS.

As specified in clause 6.3, subtitle streams which include a display definition segment shall be distinguished from those that have been coded in accordance with ETSI EN 300 743 (V1.2.1) [5], by the use of HDTV-specific or UHDTV-specific subtitling_type values in the subtitling descriptor signalled in the PMT for that service. This provides a means whereby legacy SDTV-only decoders should ignore streams which include a display definition segment.

A subtitle stream shall not convey both a subtitle service which includes a DDS and one that does not; in this case the subtitle services shall be carried in separate streams and on separate PIDs. The syntax of the DDS is shown in table 8.
