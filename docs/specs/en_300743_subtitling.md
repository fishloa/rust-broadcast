# ETSI EN 300 743 v1.6.1 — DVB Subtitling (wire-structure reference)

Reference transcription of the DVB subtitling segment syntax: the PES
`PES_data_field` carries one or more `subtitling_segment`s — page composition,
region composition, CLUT definition, object data, display definition, disparity
signalling, alternative CLUT, end-of-display-set.

**Source:** `specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf` §7.2
(PDF pp. 27–53 + the pixel-code-string tables on p. 62). Transcribed via
**BlazeDocs OCR** (the table oracle — `pdftotext` is never used for tables here),
spot-checked against the PDF render.

> **Status:** forward-reference only — no parser implemented yet. When a DVB
> subtitling decoder is built, type these segments (symmetric Parse/Serialize)
> and add `spec_tables/*.toml` drift-guards for the coded enums (segment_type,
> object_coding_method, pixel-depth, region_level_of_compatibility, …).

---

Table 3: PES data field

|  Syntax | Size | Type  |
| --- | --- | --- |
|  PES_data_field() { |  |   |
|  data_identifier | 8 | bslbf  |
|  subtitle_stream_id | 8 | bslbf  |
|  while (next_bits(8) == '0000 1111') { |  |   |
|  subtitling_segment() |  |   |
|  } |  |   |
|  end_of_PES_data_field_marker | 8 | bslbf  |
|  } |  |   |

Semantics:

data_identifier: For DVB subtitle streams the data_identifier field shall be coded with the value 0x20.

subtitle_stream_id: This identifies the subtitle stream in this PES packet. A DVB subtitling stream shall be identified by the value 0x00.

subtitling_segment(): One or more subtitling segments, as defined in clause 7.2, can be included in a single PES data field. Each subtitling_segment starts with the sync byte of '0000 1111'. The number of subtitling segments contained in the PES packet is not signalled explicitly.

end_of_PES_data_field_marker: An 8-bit field with fixed contents '1111 1111'.

## 6.3 Carriage and signalling in the transport stream

The subtitling stream PES layer shall be carried in the MPEG-2 Transport Stream as specified in ISO/IEC 13818-1 [1].

Table 4 specifies the parameters of the Transport Stream that shall be used to transport subtitle streams.

Table 4: TS carriage of subtitle streams

|  stream_type in the PMT | Set to '0x06' indicating "PES packets containing private data".  |
| --- | --- |

For each subtitle service a subtitling_descriptor as defined in ETSI EN 300 468 [2] shall signal the properties of the subtitle service in the PMT of the Transport Stream carrying that subtitle service.

The subtitling_type field in the subtitling_descriptor shall be set according to the subtitle service properties and features used in the subtitle service, as shown in table 5. The value of subtitling_type implicitly signals the version of the present document with which the subtitle service is compliant.

The subtitling_type value shall be set to the same value as the component_type value of a DVB component descriptor as defined in ETSI EN 300 468 [2] when the stream_content field of that descriptor is equal to '0x3'. Due to the evolution of the present document, features have been added to each new version. Obviously, features introduced in any version of the present document will not be supported by IRDs that were designed to be compliant with an earlier version of the specification, hence the subtitle service shall use a value of subtitling_type corresponding to the associated service, and should use only those features, i.e. segment types and ODS coding types, that were specified in the corresponding version of the present document. Subtitle services that choose not to follow this recommendation could face issues of incompatibility with legacy subtitle decoders that might not be robust against the presence of unknown or unsupported subtitling features in the subtitle service.

IRDs shall ignore subtitle services signalled with a subtitling_type that they do not support.

NOTE: It is known that some early implementations of subtitle decoders might not ignore nor be robust against the presence of unsupported subtitling_types in subtitle bitstreams.

Table 5 lists the features of the present document that are not recommended to be used in subtitle services that are provided in accordance with a particular version of the present document, which is implicitly signalled by the subtitling_type field in the subtitling_descriptor in the PMT.

Table 5: Subtitling type usage

|  Subtitling type in the subtitling_descriptor (see ETSI EN 300 468 [2]) | ETSI EN 300 743 version compliance | Indicative service compatibility | Features that are not recommended for the subtitle service  |
| --- | --- | --- | --- |
|  0x10-0x13, 0x20-0x23 | V1.1.1, V1.2.1 | SDTV | DDS, DSS, ACS, ODS object coding type = '2'  |
|  0x14, 0x24 | V1.3.1 | HDTV, UHDTV 1 | DSS, ACS, ODS object coding type = '2'  |
|  0x15, 0x25 | V1.4.1, V1.5.1 | 3DTV | ACS, ODS object coding type = '2'  |
|  0x16, 0x26 | V1.6.1 | HDTV 2, UHDTV | None  |
|  NOTE 1: The subtitle service may use only the CLUT definition segment (CDS) to define the available subtitle colours within the Recommendation ITU-R BT.601 [3] colour system. NOTE 2: The subtitle service may use ODS object coding type = '2' but in that case decoders compliant with V1.5.1 or earlier of the present document will not be able to decode the subtitles.  |   |   |   |

The subtitling_descriptor shall indicate the page id values of the segments needed to decode that subtitle service. The page id of segments with data specific to that service is referred to as the **composition page id**, while the page id of segments with shared data is referred to as the **ancillary page id**.

Version 1.6.1 of the present document introduces two new features that could, in principle, also be used with non-UHDTV service types. These features are progressive-scan bitmap objects and the alternative CLUT segment. The principle of decoder compatibility implies that if the service provider intends to maintain interoperability with existing decoders supporting an earlier version of the present document, then the new features of the later version of the present document shall not be used.

In other words, a DVB service may include subtitles with capabilities signalled with a subtitling_type that indicates a lower level of indicative service compatibility than would be expected with the associated service.

For example, a UHDTV service could include subtitle streams that do not use the new features introduced in V1.6.1, and can therefore be signalled using subtitling types 0x14 and/or 0x24, if the service provider chooses to target UHDTV IRDs with subtitle decoders that are compliant with ETSI EN 300 743 (V1.3.1) [6], ETSI EN 300 743 (V1.4.1) [7] or ETSI EN 300 743 (V1.5.1) [8] of the present document. However the service provider should bear in mind that there might be unpredictable results with the positioning of such subtitles on the screen with some UHDTV IRDs.

Conversely, if a service provider wishes to deploy progressively-coded subtitles (with ODS object coding type = '2'), subtitling type 0x16 or 0x26 shall be signalled, even if the service is not a UHDTV service.

# 7 Subtitling service data specification

## 7.1 Introduction

The present clause contains the specification of the syntax and semantics of the subtitling segment, and all subtitling segment types, in clause 7.2.

Clause 7.3 contains the specification of interoperability points for subtitle services and decoders.

## 7.2 Syntax and semantics of the subtitling segment

### 7.2.0 General

#### 7.2.0.1 Segment syntax

The basic syntactical element of subtitle streams is the "segment". It forms the common format shared amongst all elements of this subtitling specification. A segment shall be encoded as described in table 6.

Table 6: Generic subtitling segment

|  Syntax | Size | Type  |
| --- | --- | --- |
|  subtitling_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  segment_data_field() |  |   |
|  } |  |   |

Semantics:

sync_byte: An 8-bit field that shall be coded with the value '0000 1111'. Inside a PES packet, decoders can use the sync_byte to verify synchronization when parsing segments based on the segment_length, so as to determine transport packet loss.

segment_type: This indicates the type of data contained in the segment data field. Table 7 lists the segment_type values defined in the present document. Segment types that are not recognized or supported shall be ignored, without impacting the decoding of all recognized and supported segment types contained in the subtitling PES packet.

NOTE: It is known that some early implementations of subtitle decoders might not be robust against the presence of unsupported segment types in subtitle bitstreams.

Table 7: Segment types

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

Table 8: Display definition segment

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

Table 9: Page composition segment

|  Syntax | Size | Type  |
| --- | --- | --- |
|  page_composition_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  page_time_out | 8 | uimsbf  |
|  page_version_number | 4 | uimsbf  |
|  page_state | 2 | bslbf  |
|  reserved | 2 | bslbf  |
|  while (processed_length < segment_length) { |  |   |
|  region_id | 8 | bslbf  |
|  reserved | 8 | bslbf  |
|  region_horizontal_address | 16 | uimsbf  |
|  region_vertical_address | 16 | uimsbf  |
|  } |  |   |
|  } |  |   |

# Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x10, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

page_time_out: The period, expressed in seconds, after which a page instance is no longer valid and consequently shall be erased from the screen, should it not have been redefined before that. The time-out period starts when the page instance is first displayed. The page_time_out value applies to each page instance until its value is redefined. The purpose of the time-out period is to avoid a page instance remaining on the screen "for ever" if the IRD happens to have missed the redefinition or deletion of the page instance. The time-out period does not need to be counted very accurately by the IRD: a reaction accuracy of  $-0/+5$  s is accurate enough.

page_version_number: The version of this page composition segment. When any of the contents of this page composition segment change, this version number is incremented (modulo 16).

page_state: This field signals the status of the subtitling page instance described in this page composition segment. The values of the page_state are defined in table 10.

Table 10: Page state

|  Value | Page state | Effect on page | Comments  |
| --- | --- | --- | --- |
|  00 | normal case | page update | The display set contains only the subtitle elements that are changed from the previous page instance.  |
|  01 | acquisition point | page refresh | The display set contains all subtitle elements needed to display the next page instance.  |
|  10 | mode change | new page | The display set contains all subtitle elements needed to display the new page.  |
|  11 | reserved |  | Reserved for future use.  |

If the page state is "mode change" or "acquisition point", then the display set shall contain a region composition segment for each region used in this epoch.

processed_length: The total number of bytes that have already been processed following the segment_length field.

region_id: This uniquely identifies a region within a page. Each identified region is displayed in the page instance defined in this page composition. Regions shall be listed in the page_composition_segment in the order of ascending region_vertical_address field values.

region_horizontal_address: This specifies the horizontal address of the top left pixel of this region. The left-most pixel of the active pixels has horizontal address zero, and the pixel address increases from left to right.

region_vertical_address: This specifies the vertical address of the top line of this region. The top line of the frame is line zero, and the line address increases by one within the frame from top to bottom.

NOTE: All addressing of pixels is based on a frame of M pixels horizontally by N scan lines vertically. These numbers are independent of the aspect ratio of the picture; on a 16:9 display a pixel looks a bit wider than on a 4:3 display. In some cases, for instance a logo, this may lead to unacceptable distortion. Separate data may be provided for presentation on each of the different aspect ratios. The subtitling descriptor signals whether the associated subtitle data can be presented on any display or on displays of specific aspect ratio only.

## 7.2.3 Region composition segment

The region composition for a specific region is carried in region_composition_segments. The region composition contains a list of objects; the listed objects shall be positioned in such a way that they do not overlap.

If an object is added to a region in case of a page update, new pixel data will overwrite either the background colour of the region or "old objects". The programme provider shall take care that the new pixel data overwrites only information that needs to be replaced, but also that it overwrites all pixels in the region that are not to be preserved. Note that a pixel is either defined by the background colour, or by an "old" object or by a "new" object; if a pixel is overwritten none of its previous definition is retained.

Table 11 shows the syntax of the region composition segment.

Table 11: Region composition segment

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

Table 12: Region level of compatibility

|  Value | Minimum CLUT type  |
| --- | --- |
|  0x00 | reserved  |
|  0x01 | 2-bit/entry CLUT required  |
|  0x02 | 4-bit/entry CLUT required  |
|  0x03 | 8-bit/entry CLUT required  |
|  0x04..0x07 | reserved  |

If the decoder does not support the specified minimum requirement for the type of CLUT, then this region shall not be displayed, even though some other regions, requiring a lesser type of CLUT, may be presented.

region_depth: Identifies the intended pixel depth for this region as defined in table 13.

Table 13: Intended region pixel depth

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

Table 14: Object type

|  Value | Object type  |
| --- | --- |
|  0x00 | basic_object, bitmap  |
|  0x01 | basic_object, character  |
|  0x02 | composite_object, string of characters  |
|  0x03 | reserved  |

objectprovider_flag: A 2-bit flag indicating how this object is provided, as defined in table 15.

Table 15: Object provider flag

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

Table 16: CLUT definition segment

|  Syntax | Size | Type  |
| --- | --- | --- |
|  CLUT_definition_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  CLUT_id | 8 | bslbf  |
|  CLUT_version_number | 4 | uimsbf  |
|  reserved | 4 | bslbf  |
|  while (processed_length < segment_length) { |  |   |
|  CLUT_entry_id | 8 | bslbf  |
|  2-bit/entry_CLUT_flag | 1 | bslbf  |
|  4-bit/entry_CLUT_flag | 1 | bslbf  |
|  8-bit/entry_CLUT_flag | 1 | bslbf  |
|  reserved | 4 | bslbf  |
|  full_range_flag | 1 | bslbf  |
|  if full_range_flag == '1' { |  |   |
|  Y-value | 8 | bslbf  |
|  Cr-value | 8 | bslbf  |
|  Cb-value | 8 | bslbf  |
|  T-value | 8 | bslbf  |
|  } else { |  |   |
|  Y-value | 6 | bslbf  |
|  Cr-value | 4 | bslbf  |
|  Cb-value | 4 | bslbf  |
|  T-value | 2 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |

Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x12, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

CLUT_id: Uniquely identifies within a page the CLUT family whose data is contained in this CLUT_definition_segment field.

CLUT_version_number: Indicates the version of this segment data. When any of the contents of this segment change this version number is incremented (modulo 16).

processed_length: The total number of bytes that have already been processed following the segment_length field.

CLUT_entry_id: Specifies the entry number of the CLUT. The first entry of the CLUT has entry number zero.

2-bit/entry_CLUT_flag: If set to '1', this indicates that this CLUT value is to be loaded into the identified entry of the 2-bit/entry CLUT. This option shall not be used when the CDS accompanies an alternative CLUT segment (ACS).

4-bit/entry_CLUT_flag: If set to '1', this indicates that this CLUT value is to be loaded into the identified entry of the 4-bit/entry CLUT. This option shall not be used when the CDS accompanies an alternative CLUT segment (ACS).

8-bit/entry_CLUT_flag: If set to '1', this indicates that this CLUT value is to be loaded into the identified entry of the 8-bit/entry CLUT. This option shall be used when the CDS accompanies an alternative CLUT segment (ACS).

Only one N-bit/entry_CLUT_flag shall be set to 1 per CLUT_entry_id and its associated Y-, Cr-, Cb- and T-values.

full_range_flag: If set to '1', this indicates that the Y_value, Cr_value, Cb_value and T_value fields have the full 8-bit resolution. If set to '0', then these fields contain only the most significant bits.

Y_value: The Y output value of the CLUT for this entry. A value of zero in the Y_value field signals full transparency. In that case the values in the Cr_value, Cb_value and T_value fields are irrelevant and shall be set to zero.

NOTE 1: Implementers should note that $Y=0$ is disallowed in Recommendation ITU-R BT.601 [3]. This condition should be recognized and mapped to a legal value (e.g. $Y=16d$) before conversion to RGB values in a decoder.

Cr_value: The Cr output value of the CLUT for this entry.

Cb_value: The Cb output value of the CLUT for this entry.

NOTE 2: Y, Cr and Cb have meanings as defined in Recommendation ITU-R BT.601 [3] and in Recommendation ITU-R BT.656-4 [4].

NOTE 3: Note that, whilst this subtitling specification defines CLUT entries in terms of Y, Cr, Cb and T values, the standard interface definition of digital television (Recommendation ITU-R BT.656-4 [4]) presents co-sited sample values in the order Cb,Y,Cr. Failure to correctly interpret the rendered bitmap image in terms of Recommendation ITU-R BT.656-4 [4] may result in incorrect colours and chrominance mistiming.

T_value: The Transparency output value of the CLUT for this entry. A value of zero identifies no transparency. The maximum value plus one would correspond to full transparency. For all other values the level of transparency is defined by linear interpolation.

Full transparency is acquired through a value of zero in the Y_value field.

NOTE 4: Decoder models for the translation of pixel-codes into Y, Cr, Cb and T values are depicted in clause 9. Default contents of the CLUT are specified in clause 10.

NOTE 5: The colour for each CLUT entry can be redefined. There is no need for CLUTs with fixed contents as every CLUT has default contents, see clause 10.

## 7.2.5 Object data segment

## 7.2.5.0 General

The object_data_segment contains the data of an object. For graphical objects with the object_coding_method setting of "coding of pixels" the following applies:

- an object may be interlaced, with a top field and a bottom field or a top field that is repeated as the bottom field, or it may be progressive, with a single field of object data;
- the first pixel of the first line of the top field is the top left pixel of the object;
- the first pixel of the first line of the bottom field is the most left pixel on the second line of the object;
- for interlaced objects:

- the same object_data_segment shall carry a pixel-data_sub-block for both the top field and the bottom field;
- if a segment carries no data for the bottom field, i.e. the bottom_field_data_block_length contains the value '0x0000', then the pixel-data_sub-block for the top field shall apply for the bottom field also.

The object_data_segment is defined as shown in table 17.

Table 17: Object data segment

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

Table 18: Object coding method

|  Value | Object coding method  |
| --- | --- |
|  0x0 | coding of pixels (see note 1)  |
|  0x1 | coded as a string of characters  |
|  0x2 | progressive coding of pixels (see note 2)  |
|  0x3 | reserved  |
|  NOTE 1: The value 0x0 indicates interlaced coding of pixels, the only method available for coding of pixels prior to version V1.6.1 of the present document. NOTE 2: This object coding method is introduced in version 1.6.1 of the present document, hence subtitle decoders that are compliant with an earlier version of the present document will be unable to process this mode.  |   |

non_modifying_colour_flag: If set to '1' this indicates that the CLUT entry value '1' is a non modifying colour. When the non modifying colour is assigned to an object pixel, then the pixel of the underlying region background or object shall not be modified. This can be used to create "transparent holes" in objects.

top_field_data_block_length: Specifies the number of bytes contained in the pixel-data_sub-blocks for the top field.

bottom_field_data_block_length: Specifies the number of bytes contained in the data_sub-block for the bottom field.

pixel-data_sub-block(): Contains the run-length encoded data for each field of the object. Its structure is defined in clause 7.2.5.1.

processed_length: The number of bytes from the field(s) within the while-loop that have been processed by the decoder.

stuffing_length: The value is not signalled but it can be calculated from other fields and shall be either zero or one.

NOTE: In earlier versions of the present document the presence or absence of the 8_stuff_bits field was determined by an undefined wordaligned() function which created an ambiguity. This was replaced by the stuffing_length value to remove the ambiguity.

Some legacy subtitle encoders may operate differently to the recommended behaviour defined below in table 19. However in all cases subtitle decoders shall calculate the stuffing_length value using the following equation:

$$
\text{stuffing_length} = \text{segment_length} - 7 - \text{top_field_data_block_length} - \text{bottom_field_data_block_length}
$$

Subtitle encoders should add an 8_stuff_bits field only if the sum of top_field_data_block_length and bottom_field_data_block_length is an even number. Therefore the segment_length field will always be set to an even number. The recommended encoder behaviour is summarized in table 19.

Table 19: Recommended encoding of object_data_segment

|  top_field_data_block_length + bottom_field_data_block_length | 8_stuff_bits | stuffing_length (implied) | segment_length  |
| --- | --- | --- | --- |
|  Is an Odd number | Not present | 0 | 7 + stuffing_length + top_field_data_block_length + bottom_field_data_block_length (Always an even number)  |
|  Is an Even number | Present | 1  |   |

8_stuff_bits: If present, this field shall be coded as '0000 0000'.

number_of_codes: Specifies the number of character codes in the string.

character_code: Specifies a character through its index number in a character table, the definition of which is not included in the present document. The specification and provision of such a character code table is part of the local agreement between the subtitle service provider and IRD manufacturer that is needed to put this mode of subtitles into operation.

progressivepixel_block(): Contains the data for the progressively coded object. Its structure is defined in clause 7.2.5.3.

## 7.2.5.1 Pixel-data sub-block

The pixel-data sub-block structure is used with object coding method 0x0, i.e. "coding of pixels".

For each object the pixel-data sub-block for the top field and the pixel-data sub-block for the bottom field shall be carried in the same object_data_segment. If this segment carries no data for the bottom field, i.e. the bottom_field_data_block_length contains the value '0x0000', then the data for the top field shall be valid for the bottom field also.

NOTE: This effectively forbids an object from having a height of only one TV picture line. Isolated objects of this height would be liable to suffer unpleasant flicker effects at the TV display frame rate when displayed on an interlaced display.

Table 20 defines the syntax of the pixel-data sub-block structure.

Table 20: Pixel-data sub-block

|  Syntax | Size | Type  |
| --- | --- | --- |
|  pixel-data_sub-block() { |  |   |
|  data_type | 8 | bslbf  |
|  if data_type =='0x10' { |  |   |
|  repeat { |  |   |
|  2-bit/pixel_code_string() |  |   |
|  } until (end of 2-bit/pixel_code_string) |  |   |
|  while (!bytealigned()) |  |   |
|  2_stuff_bits | 2 | bslbf  |
|  if data_type =='0x11' { |  |   |
|  repeat { |  |   |
|  4-bit/pixel_code_string() |  |   |
|  } until (end of 4-bit/pixel_code_string) |  |   |
|  if (!bytealigned()) |  |   |
|  4_stuff_bits | 4 | bslbf  |
|  } |  |   |
|  } |  |   |
|  if data_type =='0x12' { |  |   |
|  repeat { |  |   |
|  8-bit/pixel_code_string() |  |   |
|  } until (end of 8-bit/pixel_code_string) |  |   |
|  } |  |   |
|  if data_type =='0x20' |  |   |
|  2_to_4-bit_map-table | 16 | bslbf  |
|  if data_type =='0x21' |  |   |
|  2_to_8-bit_map-table | 32 | bslbf  |
|  if data_type =='0x22' |  |   |
|  4_to_8-bit_map-table | 128 | bslbf  |
|  } |  |   |

## Semantics:

data_type: Identifies the type of information contained in the pixel-data_sub-block according to table 21.

Table 21: Data type

|  Value | data_type  |
| --- | --- |
|  0x10 | 2-bit/pixel code string  |
|  0x11 | 4-bit/pixel code string  |
|  0x12 | 8-bit/pixel code string  |
|  0x20 | 2_to_4-bit_map-table data  |
|  0x21 | 2_to_8-bit_map-table data  |
|  0x22 | 4_to_8-bit_map-table data  |
|  0xF0 | end of object line code  |
|  NOTE: All other values are reserved.  |   |

---

The data types 2-bit/pixel code string, 4-bit/pixel code string, and 8-bit/pixel code string are defined in clause 7.2.5.2.

A code '0xF0' = "end of object line code" shall be included after every series of code strings that together represent one line of the object.

2_to_4-bit_map-table: Specifies how to map the 2-bit/pixel codes on a 4-bit/entry CLUT by listing the 4 entry numbers of 4-bits each; entry number 0 first, entry number 3 last.
2_to_8-bit_map-table: Specifies how to map the 2-bit/pixel codes on an 8-bit/entry CLUT by listing the 4 entry numbers of 8-bits each; entry number 0 first, entry number 3 last.
4_to_8-bit_map-table: Specifies how to map the 4-bit/pixel codes on an 8-bit/entry CLUT by listing the 16 entry numbers of 8-bits each; entry number 0 first, entry number 15 last.
2_stuff_bits: Two stuffing bits that shall be coded as '00'.
4_stuff_bits: Four stuffing bits that shall be coded as '0000'.

bytealigned(): function is true if current position is aligned to whole byte boundary from the start of the pixel-data_sub-block().

## 7.2.5.2 Syntax and semantics of the pixel code strings

## 7.2.5.2.1 2-bits per pixel code

Table 22 defines the syntax of the 2-bits per pixel code string.

Table 22: 2-bits per pixel code string

|  Syntax | Size | Type  |
| --- | --- | --- |
|  2-bit/pixel_code_string() { |  |   |
|  if (next_bits(2) != '00') { |  |   |
|  2-bitpixel-code | 2 | bslbf  |
|  } else { |  |   |
|  2-bit_zero | 2 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if (switch_1 == '1') { |  |   |
|  run_length_3-10 | 3 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } else { |  |   |
|  switch_2 | 1 | bslbf  |
|  if (switch_2 == '0') { |  |   |
|  switch_3 | 2 | bslbf  |
|  if (switch_3 == '10') { |  |   |
|  run_length_12-27 | 4 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } |  |   |
|  if (switch_3 == '11') { |  |   |
|  run_length_29-284 | 8 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |

## Semantics:

2-bitpixel-code: A 2-bit code, specifying the pseudo-colour of a pixel as either an entry number of a CLUT with four entries or an entry number of a map-table.
2-bit_zero: A 2-bit field filled with '00'.

switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_3-10: Number of pixels minus 3 that shall be set to the pseudo-colour defined next.

switch_2: A 1-bit switch. If set to '1', it signals that one pixel shall be set to pseudo-colour (entry) '00', else it indicates the presence of the following fields.

switch_3: A 2-bit switch that may signal one of the properties listed in table 23.

Table 23: switch_3 for 2-bits per pixel code

|  Value | Meaning  |
| --- | --- |
|  00 | end of 2-bit/pixel_code_string  |
|  01 | two pixels shall be set to pseudo colour (entry) '00'  |
|  10 | the following 6 bits contain run length coded pixel data  |
|  11 | the following 10 bits contain run length coded pixel data  |

run_length_12-27: Number of pixels minus 12 that shall be set to the pseudo-colour defined next.

run_length_29-284: Number of pixels minus 29 that shall be set to the pseudo-colour defined next.

## 7.2.5.2.2 4-bits per pixel code

Table 24 defines the syntax of the 4-bits per pixel code string.

Table 24: 4-bits per pixel code string

|  Syntax | Size | Type  |
| --- | --- | --- |
|  4-bit/pixel_code_string() { |  |   |
|  if (next_bits(4) != '0000') { |  |   |
|  4-bit_pixel-code | 4 | bslbf  |
|  } else { |  |   |
|  4-bit_zero | 4 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if (switch_1 == '0') { |  |   |
|  if (next_bits(3) != '000') |  |   |
|  run_length_3-9 | 3 | uimsbf  |
|  Else |  |   |
|  end_of_string_signal | 3 | bslbf  |
|  } else { |  |   |
|  switch_2 | 1 | bslbf  |
|  if (switch_2 == '0') { |  |   |
|  run_length_4-7 | 2 | bslbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } else { |  |   |
|  switch_3 | 2 | bslbf  |
|  if (switch_3 == '10') { |  |   |
|  run_length_9-24 | 4 | uimsbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } |  |   |
|  if (switch_3 == '11') { |  |   |
|  run_length_25-280 | 8 | uimsbf  |
|  4-bit_pixel-code | 4 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |

# Semantics:

4-bitpixel-code: A 4-bit code, specifying the pseudo-colour of a pixel as either an entry number of a CLUT with sixteen entries or an entry number of a map-table.

4-bit_zero: A 4-bit field filled with '0000'.

switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_3-9: Number of pixels minus 2 that shall be set to pseudo-colour (entry) '0000'.

end_of_string_signal: A 3-bit field filled with '000'. The presence of this field, i.e. next_bits(3) == '000', signals the end of the 4-bit/pixel_code_string.

switch_2: A 1-bit switch. If set to '0', it signals that the following 6-bits contain run-length coded pixel-data, else it indicates the presence of the following fields.

switch_3: A 2-bit switch that may signal one of the properties listed in table 25.

Table 25: switch_3 for 4-bits per pixel code

|  Value | Meaning  |
| --- | --- |
|  00 | 1 pixel shall be set to pseudo-colour (entry) '0000'  |
|  01 | 2 pixels shall be set to pseudo-colour (entry) '0000'  |
|  10 | the following 8 bits contain run-length coded pixel-data  |
|  11 | the following 12 bits contain run-length coded pixel-data  |

run_length_4-7: Number of pixels minus 4 that shall be set to the pseudo-colour defined next.

run_length_9-24: Number of pixels minus 9 that shall be set to the pseudo-colour defined next.

run_length_25-280: Number of pixels minus 25 that shall be set to the pseudo-colour defined next.

## 7.2.5.2.3 8-bits per pixel code

Table 26 defines the syntax of the 8-bits per pixel code string.

Table 26: 8-bits per pixel code string

|  Syntax | Size | Type  |
| --- | --- | --- |
|  8-bit/pixel_code_string() { |  |   |
|  if (next_bits(8) != '0000 0000') { |  |   |
|  8-bitpixel-code | 8 | bslbf  |
|  } else { |  |   |
|  8-bit_zero | 8 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if switch_1 == '0' { |  |   |
|  if next_bits(7) != '000 0000' |  |   |
|  run_length_1-127 | 7 | uimsbf  |
|  else |  |   |
|  end_of_string_signal | 7 | bslbf  |
|  } else { |  |   |
|  run_length_3-127 | 7 | uimsbf  |
|  8-bitpixel-code | 8 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |

# Semantics:

8-bitpixel-code: An 8-bit code, specifying the pseudo-colour of a pixel as an entry number of a CLUT with 256 entries.

8-bit_zero: An 8-bit field filled with '0000 0000'.

switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_1-127: Number of pixels that shall be set to pseudo-colour (entry) '0x00'.

end_of_string_signal: A 7-bit field filled with '000 0000'. The presence of this field, i.e. next_bits(7) == '000 0000', signals the end of the 8-bit/pixel_code_string.

run_length_3-127: Number of pixels that shall be set to the pseudo-colour defined next. This field shall not have a value of less than three.

## 7.2.5.3 Progressive pixel block

The progressive pixel block format is used with object coding method 0x2, i.e. "progressive coding of pixels".

This object coding method is introduced in V1.6.1 of the present document, hence it shall not be used in systems where subtitle decoders are in operation that were designed to be compliant with ETSI EN 300 743 (V1.5.1) [8] or an earlier version.

Subtitle streams with progressive object coding type shall use subtitling_type value 0x16 or 0x26 in the subtitling descriptor signalled in the PMT for the service in which they are carried. Subtitle streams that have subtitling_type value not equal to either 0x16 or 0x26 shall not use the progressive coding object type. This ensures that IRDs that are compliant with V1.5.1 or an earlier version of the present document should not be presented with subtitle services that use object coding method 0x2.

The progressive pixel block format shall not be used to carry interlace-scan subtitle segments.

Progressively coded subtitle bitmaps shall be carried in the zlib datastream format, as defined in IETF RFC 1950 [14]. This format applies the DEFLATE compression method as defined by IETF RFC 1951 [15]. The parameters for zlib and DEFLATE usage shall be the same as those applied in the Portable Network Graphics (PNG) format [16] with "Compression method 0" applied to the sequence of filtered scanlines, without any further PNG format overhead, i.e. without the PNG "chunk" structure.

The syntax of the progressive pixel block is shown in table 27.

Table 27: Progressive pixel block

|  Syntax | Size | Type  |
| --- | --- | --- |
|  progressivepixel_block() { |  |   |
|  bitmap_width | 16 | uimsbf  |
|  bitmap_height | 16 | uimsbf  |
|  compressed_data_block_length | 16 | uimsbf  |
|  for (i=0; i<compressed_data_block_length; i++) { |  |   |
|  compressed_bitmap_data_byte | 8 | bslbf  |
|  } |  |   |
|  } |  |   |

# Semantics:

bitmap_width: This field shall indicate the width of the subtitle bitmap image in pixels.

bitmap_height: This field shall indicate the height of the subtitle bitmap image in pixels.

compressed_data_block_length: This field shall indicate the number of compressed_bitmap_data_byte following this field.

compressed_bitmap_data_byte: This field is formed of the sequence of bytes of the subtitle bitmap image in compressed form, which is according to the zlib container format [14], in the same way as is specified for the Portable Network Graphics (PNG) format [16]. This format applies the DEFLATE compression algorithm [15]. The compressed bitmap data shall consist of the raw zlib datastream and shall not contain any PNG format overhead such as chunk headers or chunk CRC values.

Annex E provides an informative description of the conversion process for a suitably coded PNG file to be converted into a progressively-coded subtitle bitmap.

# 7.2.6 End of display set segment

The end_of_display_set_segment provides an explicit indication to the decoder that transmission of a display set is complete. The end_of_display_set_segment shall be inserted into the stream as the last segment for each display set. It shall be present for each subtitle service in a subtitle stream, although decoders need not take advantage of this segment and may apply other strategies to determine when they have sufficient information from a display set to commence decoding. The syntax of the end_of_display_set_segment is shown in table 28.

Table 28: End of display set segment

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

Table 29: Disparity signalling segment

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

Table 30: disparity_shift_update_sequence

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

Table 31: Alternative CLUT segment

|  Syntax | Size | Type  |
| --- | --- | --- |
|  alternative_CLUT_segment() { |  |   |
|  sync_byte | 8 | bslbf  |
|  segment_type | 8 | bslbf  |
|  page_id | 16 | bslbf  |
|  segment_length | 16 | uimsbf  |
|  CLUT_id | 8 | bslbf  |
|  CLUT_version_number | 4 | uimsbf  |
|  reserved_zero_future_use | 4 | bslbf  |
|  CLUT_parameters() | 16 | bslbf  |
|  while (processed_length < segment_length) { |  |   |
|  If (output_bit_depth == 0) { |  |   |
|  luma-value | 8 | uimsbf  |
|  chroma1-value | 8 | uimsbf  |
|  chroma2-value | 8 | uimsbf  |
|  T-value | 8 | uimsbf  |
|  } |  |   |
|  If (output_bit_depth == 1) { |  |   |
|  luma-value | 10 | uimsbf  |
|  chroma1-value | 10 | uimsbf  |
|  chroma2-value | 10 | uimsbf  |
|  T-value | 10 | uimsbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |

# Semantics:

sync_byte: This field shall contain the value '0000 1111'.

segment_type: This field shall contain the value 0x16, as listed in table 7.

page_id: The page_id identifies the subtitle service of the data contained in this subtitling_segment. Segments with a page_id value signalled in the subtitling descriptor as the composition page id, carry subtitling data specific for one subtitle service. Accordingly, segments with the page_id signalled in the subtitling descriptor as the ancillary page id, carry data that may be shared by multiple subtitle services.

segment_length: This field shall indicate the number of bytes contained in the segment following the segment_length field.

CLUT_id: This field identifies within a page the CLUT family whose data is contained in this alternative_CLUT_segment field. Its value shall be the same as for the CLUT_id contained in the CDS of the same subtitle service.

CLUT_version_number: Indicates the version of this segment data. When any of the contents of this segment change this version number is incremented (modulo 16).

reserved_zero_future_use: These bits are reserved for future use. They shall be set to the value  $0 \times 0$ .

CLUT_parameters: This 16-bit field has the syntax as shown in table 32.

Table 32: CLUT parameters

|  Syntax | Size | Type  |
| --- | --- | --- |
|  CLUT_parameters() { |  |   |
|  CLUT_entry_max_number | 2 | bslbf  |
|  colour_component_type | 2 | bslbf  |
|  output_bit_depth | 3 | bslbf  |
|  reserved_zero_future_use | 1 | bslbf  |
|  dynamic_range_and_colour_gamut | 8 | bslbf  |
|  } |  |   |

# Semantics:

CLUT_entry_max_number: This two-bit field shall indicate the maximum number of CLUT entries. A value of '0' corresponds to a maximum number of 256 entries. All other values are reserved. Any number of CLUT entries can be provided, up to the maximum number.

colour_component_type: This two-bit field shall indicate the type of colour coding used in the chroma1-value and chroma2-value fields. A value of '0' corresponds to colour coding type YCbCr, whereby chroma1-value is Cb and chroma2-value is Cr. All other values are reserved.

output_bit_depth: This three-bit field shall indicate the bit-depth of the output of each component, as shown in table 33. If the graphics plane of the IRD has a bit-depth different from the output_bit_depth setting, then the IRD shall perform the appropriate conversion for each component value of the CLUT.

Table 33: Output bit-depth coding

|  Value | Output bit-depth  |
| --- | --- |
|  0x0 | 8  |
|  0x1 | 10  |
|  0x2 - 0x7 | Reserved  |

reserved_zero_future_use: This bit is reserved for future use. It shall be set to the value 0.

dynamic_range_and_colour_gamut: This eight-bit field shall be coded according to one of the entries in table 34.

Table 34: Dynamic range and colour gamut coding

|  Value | Dynamic range and colour gamut  |
| --- | --- |
|  0x00 | SDR; ITU-R BT.709 [10]  |
|  0x01 | SDR; ITU-R BT.2020-2 [11]  |
|  0x02 | HDR; ITU-R BT.2100-1 [12] PQ  |
|  0x03 | HDR; ITU-R BT.2100-1 [12] HLG  |
|  0x04 - 0xFF | Reserved  |

luma-value: This field indicates the luma output value of the CLUT entry.

chroma1-value: This field indicates the first chroma output value of the CLUT entry.

chroma2-value: This field indicates the second chroma output value of the CLUT entry.

---

## §7.2.5.2 — pixel-code strings

# 11 Structure of the pixel code strings (informative)

The structure of the 2-bit/pixel_code_string is shown in table 42.

Table 42: 2-bit/pixel_code_string()

|  Value | Meaning  |
| --- | --- |
|  01 | one pixel in colour 1  |
|  10 | one pixel in colour 2  |
|  11 | one pixel in colour 3  |
|  00 01 | one pixel in colour 0  |
|  00 00 01 | two pixels in colour 0  |
|  00 1L LL CC | L pixels (3..10) in colour C  |
|  00 00 10 LL LL CC | L pixels (12..27) in colour C  |
|  00 00 11 LL LL LL LL CC | L pixels (29..284) in colour C  |
|  00 00 00 | end of 2-bit/pixel_code_string  |
|  NOTE: Runs of 11 pixels and 28 pixels can be coded as one pixel plus a run of 10 pixels and 27 pixels, respectively.  |   |

The structure of the 4-bit/pixel_code_string is shown in table 43.

Table 43: 4-bit/pixel_code_string()

|  Value | Meaning  |
| --- | --- |
|  0001 | one pixel in colour 1  |
|  To | to  |
|  1111 | one pixel in colour 15  |
|  0000 1100 | one pixel in colour 0  |
|  0000 1101 | two pixels in colour 0  |
|  0000 0LLL | L pixels (3..9) in colour 0 (L>0)  |
|  0000 10LL CCCC | L pixels (4..7) in colour C  |
|  0000 1110 LLLL CCCC | L pixels (9..24) in colour C  |
|  0000 1111 LLLL LLLL CCCC | L pixels (25..280) in colour C  |
|  0000 0000 | end of 4-bit/pixel_code_string  |
|  NOTE: Runs of 8 pixels in a colour not equal to '0' can be coded as one pixel plus a run of 7 pixels.  |   |

The structure of the 8-bit/pixel_code_string is shown in table 44.

Table 44: 8-bit/pixel_code_string()

|  Value | Meaning  |
| --- | --- |
|  00000001 | one pixel in colour 1  |
|  To | to  |
|  11111111 | one pixel in colour 255  |
|  00000000 0LLLLLLL | L pixels (1-127) in colour 0 (L > 0)  |
|  00000000 1LLLLLLL CCCCCCCC | L pixels (3-127) in colour C (L > 2)  |
|  00000000 00000000 | end of 8-bit/pixel_code_string  |
