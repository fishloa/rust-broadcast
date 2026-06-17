## Table 15 — PES header constraints for carriage of DVB TTML subtitles in a PES
_§5.2.2.2.1, PDF pp. 20-20_

| Field name | Requirement |
|---|---|
| stream_id | Set to "1011 1101" (0xBD) indicating "private_stream_1". |
| PES_packet_length | Set to a value that specifies the length of the PES packet, as defined in ISO/IEC 13818-1 [1]. |
| data_alignment_indicator | Set to "1" indicating that the subtitle segments are aligned with the PES packets. |
| PTS_DTS_flags | Set to '10' to indicate the PTS is present. |
| PTS | Set to the presentation time stamp of the PES packet |
| PES_packet_data_byte | The PES_data_field specified in table 1 of the present document. |

> **Spec note:** Table 15's `PES_packet_data_byte` row literally cross-references "table 1", reproduced verbatim. The PES_data_field syntax is actually defined in **Table 16** ("PES data field", §5.2.2.2.1) of this spec — the "table 1" reference appears to be an editorial error in EN 303 560 V1.1.1.

