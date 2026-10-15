## T/AI 109.7-2024 Table 10 - AVS2 audio stream descriptor syntax (AVS2_audio_descriptor)
_T/AI 109.7-2024 clause 8.2.3.4, printed pp. 25-26 (Chinese original; free from aitisa.org.cn). Tag 211 (0xD3), TS and PS (Table 9); `stream_type` 0xD3, PES stream_id private_stream_1 `1011 1101` (8.2.2); `registration_descriptor` format_identifier 0x41565341 ('AVSA') (8.2.3.3). Minimum `descriptor_length` is 2._

| Syntax | Bits | Mnemonic |
|---|---|---|
| AVS2_audio_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| num_channels | 8 | uimsbf |
| sample_rate_index | 4 | uimsbf |
| avs_version_flag | 1 | bslbf |
| text_present_flag | 1 | bslbf |
| language_present_flag | 1 | bslbf |
| reserved | 1 | '0' |
| if (text_present_flag == 1) { text_length 8 uimsbf; for (i<text_length) { text[i] 16 bslbf } } | | |
| if (language_present_flag == 1) { language 24 bslbf } | | |
| if (avs_version_flag == 1) { | | |
| audio_codec_id | 4 | uimsbf |
| anc_data_index | 1 | bslbf |
| coding_profile | 3 | uimsbf |
| if (audio_codec_id == 0) { bitrate_index 4 uimsbf; bitstream_type 1 bslbf; reserved 3 bslbf; raw_frame_length 16 uimsbf } | | |
| resolution | 2 | uimsbf |
| reserved | 6 | bslbf |
| if (anc_data_index) { anc_data_block() } | | |
| } | | |
| for (i=0; i<N; i++) { addition_info[i] | 8 | bslbf } | | |
| } | | |

Semantics: `text[i]` 2-byte Unicode; `language` per GB/T 4880.2-2000; `sample_rate_index`, `audio_codec_id`, `anc_data_index`, `coding_profile`, `bitrate_index`, `bitstream_type`, `raw_frame_length`, `resolution`, `anc_data_block()` per GB/T 33475.3 Annex A.2 (not vendored). `avs_version_flag` "usually 1".
