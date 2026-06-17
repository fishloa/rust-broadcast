# Table 16: CLUT definition segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


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
