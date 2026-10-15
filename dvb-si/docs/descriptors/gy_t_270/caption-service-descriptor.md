## GY/T 270-2013 Table 8 - caption_service_descriptor
_GY/T 270-2013 数字电视隐藏字幕系统规范 (DTV closed caption system), clause 6.4, printed p. 6; Table 9 on p. 7. Chinese original, free NRTA download; field names verified against the rendered page. Carried in the first descriptor loop of the PMT (max 16 per PMT) and in the EIT; every programme with caption services shall carry at least one._

| Syntax | Bits | Type |
|---|---|---|
| caption_service_descriptor() { | | |
| descriptor_tag | 8 | 0x86 |
| descriptor_length | 8 | uimsbf |
| reserved | 3 | '111' |
| number_of_services | 5 | uimsbf |
| for (i=0; i<number_of_services; i++) { | | |
| language | 24 | uimsbf |
| reserved | 2 | '11' |
| caption_service_number | 6 | uimsbf |
| reserved | 1 | '1' |
| wide_aspect_ratio | 1 | bslbf |
| char_set | 6 | uimsbf |
| reserved | 8 | "1111111" (as printed) |
| } | | |
| reserved | 3 | '111' |
| caption_service_pid | 13 | uimsbf |
| } | | |

Each entry is 6 bytes (24+2+6+1+1+6+8 = 48 bits); `descriptor_length` = 1 + 6 x number_of_services + 2. `language`: 3-byte code of GB/T 4880.2-2000. `wide_aspect_ratio`: 0 = 4:3, 1 = 16:9.

### Table 9 char_set
0 GB 2312-1980; 1 GB 13000.1-2010; 2 GB 18030-2005; 3-63 reserved.

### Related wire facts (same standard)
- Clause 6.2 Table 3: caption data in a dedicated PES - stream_id 0xBD, `data_alignment_indicator` 1, PTS = display time of the matching video frame, `stream_type` 0x80.
- Clause 6.3.2: in MPEG-2/AVS/AVS+ picture user data: `user_data_start_code` 0x000001B2, `user_identifier` 0x47413934 ('GA94'), `DVB1_data()` = `user_data_type_code` 0x03 + `cc_data()`.
- Clause 6.3.3: in an H.264 SEI `user_data_registered_itu_t_t35`: `itu_t_t35_country_code` 0x26, provider code 0x0031, `user_identifier` 'GA94'.
- Clause 7.2 Table 10 `cc_data()`: reserved '1', process_cc_data_flag, zero_bit '0', cc_count(5), reserved 0xFF, then cc_count x (one_bit '1', reserved '1111', cc_valid, cc_type(2), cc_data_1, cc_data_2), marker 0xFF - bit-identical to the `cc-data` crate's `CcData`.
