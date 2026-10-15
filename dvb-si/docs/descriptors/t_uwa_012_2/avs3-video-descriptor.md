## T/UWA 012.2-2023 Table 1 - AVS3 video descriptor (older, tag 62)
_T/UWA 012.2-2023 "Bai Cheng Qian Ping" UHD A/V coding: system, clause 5.4.1, pp. 2-3 (English machine translation of the Chinese original; verified against the rendered pages). Adds tag value 62 to GB/T 17975.1-2010 Table 50 for the AVS3 video descriptor. stream_id `1111 1101` = T/AI 109.2 video, `stream_type` 0xD4 (5.2/5.3); this older form predates the 0xD1 AVS3_video_descriptor of T/AI 109.6 / GY/T 420._

| Syntax | Bits | Mnemonic |
|---|---|---|
| AVS3_video_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| profile_id | 8 | uimsbf |
| level_id | 8 | uimsbf |
| multiple_frame_rate_flag | 1 | bslbf |
| frame_rate_code | 4 | uimsbf |
| sample_precision | 3 | uimsbf |
| chroma_format | 2 | uimsbf |
| temporal_id_flag | 1 | bslbf |
| td_mode_flag | 1 | bslbf |
| library_stream_flag | 1 | uimsbf |
| reserved | 3 | bslbf |
| colour_primaries | 8 | uimsbf |
| transfer_characteristics | 8 | uimsbf |
| matrix_coefficients | 8 | uimsbf |
| if (!library_stream_flag) { | | |
| num_ref_library_stream | 7 | uimsbf |
| id_type_flag | 1 | bslbf |
| for (i=0; i<num_ref_library_stream; i++) { | | |
| if (id_type_flag) { ref_library_stream_PEID[i] | 13 | uimsbf } | | |
| else { ref_library_stream_id[i] 8 uimsbf; reserved 5 bslbf } | | |
| reserved | 3 | bslbf |
| } | | |
| } | | |
| } | | |

`id_type_flag` 1: the library bitstream is indexed by the PEID (PID) of the TS packets carrying it; 0: by the `stream_id` of its PES (when the descriptor is in a program stream it shall be 0). `ref_library_stream_PEID`: PID of the i-th library bitstream the main bitstream depends on. `library_stream_flag` 1: this ES is a library stream.
