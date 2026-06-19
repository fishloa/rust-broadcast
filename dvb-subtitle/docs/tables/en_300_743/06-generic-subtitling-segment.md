# Table 6: Generic subtitling segment

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


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
