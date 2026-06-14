# Table 31: Alternative CLUT segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


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
