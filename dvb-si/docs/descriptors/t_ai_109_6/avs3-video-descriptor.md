## AVS3 video stream descriptor (AVS3_video_descriptor)
_T/AI 109.6-2022 clause 9.3.2, Table 9 (Chinese original; free from aitisa.org.cn) and GY/T 420-2025 clause 7.3.3.2, Table 6 (NRTA free download). Tag 209 (0xD1), TS and PS (T/AI 109.6 Table 8). `stream_type` 0xD4 (T/AI 109.6 9.2.1 / GY/T 420), PES `stream_id` 0xFD, `stream_id_extension` 0x41 (main bitstream) / 0x42 (library bitstream). ETSI TS 101 154 V2.10.1 section 4.1.8: shall be preceded by a private_data_specifier_descriptor `0x41565356` ('AVSV'); the Chinese documents use a registration_descriptor (tag 0x05) with format_identifier 0x41565356 instead._

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
| library_picture_enable_flag | 1 | uimsbf |
| reserved | 2 | bslbf |
| colour_primaries | 8 | uimsbf |
| transfer_characteristics | 8 | uimsbf |
| matrix_coefficients | 8 | uimsbf |
| reserved | 8 | bslbf |
| } | | |

**Discrepancy**: GY/T 420 Table 6 (above) includes `colour_primaries` (8-byte body). T/AI 109.6 Table 9 omits that row (7-byte body) although its clause 9.3.3 semantics define `colour_primaries`. Both tables give `td_mode_flag` 1 bit; both semantics texts say 2 bits (the syntax sums to 8 bits only with 1 bit).

Table 7 (GY/T 420) / Table 10 (T/AI 109.6) `frame_rate_code`: 0000 forbidden; 0001 23.976; 0010 24 (also 23.976); 0011 25; 0100 29.97 (also 23.976); 0101 30 (also 23.976, 24, 29.97); 0110 50 (also 25); 0111 59.94 (also 23.976, 29.97); 1000 60 (also 23.976, 24, 29.97, 30, 59.94); 1001 100 (also 50); 1010 120 (also 59.94, 60, 119.88, 120); 1011-1111 reserved. "Also" = other rates allowed when `multiple_frame_rate_flag` is 1.
