# Table 3: PES data field

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


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
