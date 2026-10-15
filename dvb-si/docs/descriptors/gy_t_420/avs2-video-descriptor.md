## GY/T 420-2025 Table 3 - AVS2 video stream descriptor (AVS2_video_descriptor)
_GY/T 420-2025 clause 7.2.3, printed pp. 4-5 (Chinese original, free NRTA download). Tag 64 (0x40); `stream_type` 0xD2, `stream_id` `1110 xxxx` (Table 2); a `registration_descriptor` (tag 0x05) with format_identifier 0x41565356 ('AVSV') shall be in the PMT (clause 7.2.2)._

| Syntax | Bits | Mnemonic |
|---|---|---|
| AVS2_video_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| profile_id | 8 | uimsbf |
| level_id | 8 | uimsbf |
| extension_layer_number | 8 | uimsbf |
| multiple_frame_rate_flag | 1 | bslbf |
| frame_rate_code | 4 | uimsbf |
| AVS_still_present | 1 | bslbf |
| chroma_format | 2 | uimsbf |
| sample_precision | 3 | uimsbf |
| reserved | 5 | bslbf |
| } | | |

`extension_layer_number` is 0. `profile_id`/`level_id`/`chroma_format`/`sample_precision` are as in GY/T 299.1-2016. `frame_rate_code` Table 4: 0000 forbidden; 0001 23.976; 0010 24 (also 23.976); 0011 25; 0100 29.97 (also 23.976); 0101 30 (also 23.976, 24, 29.97); 0110 50 (also 25); 0111 59.94 (also 23.976, 29.97); 1000 60 (also 23.976, 24, 29.97, 30, 59.94); 1001 100 (also 50); 1010 120 (also 59.94, 60, 119.88, 120); 1011-1111 reserved. `AVS_still_present`: 1 = only still-picture data.
