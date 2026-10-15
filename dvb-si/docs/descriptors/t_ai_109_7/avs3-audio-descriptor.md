## T/AI 109.7-2024 Table 4 - AVS3 audio stream descriptor syntax (AVS3_audio_descriptor)
_T/AI 109.7-2024 "Information Technology - Intelligent Media Coding - Part 7: Audio Encapsulation and Transport" (Chinese original), clause 8.1.3.4, printed pp. 20-21. Free download: aitisa.org.cn (T/AI group standard). Field names are the spec's. Values verified by rendering the table pages._

Tag value 210 (0xD2), TS and PS (Table 3). `stream_type` 0xD5 (clause 8.1.2), PES `stream_id` 0xFD with `stream_id_extension` `100 1111`. `registration_descriptor` format_identifier `0x41565341` ('AVSA') (8.1.3.3). ETSI TS 101 154 V2.10.1 4.1.8.35: the descriptor shall be preceded by a private_data_specifier_descriptor `0x41565341`.

| Syntax | Bits | Mnemonic |
|---|---|---|
| AVS3_audio_descriptor() { | | |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| audio_codec_id | 4 | uimsbf |
| sampling_frequency_index | 4 | uimsbf |
| if (audio_codec_id == 0) { | | |
| anc_data_index | 1 | bslbf |
| coding_profile | 3 | uimsbf |
| bitrate_index | 4 | uimsbf |
| bitstream_type | 1 | bslbf |
| channel_number_index | 7 | uimsbf |
| raw_frame_length | 16 | uimsbf |
| } | | |
| if (audio_codec_id == 1) { | | |
| if (sampling_frequency_index == 0xf) { sampling_frequency | 24 | uimsbf |
| } | | |
| anc_data_index | 1 | bslbf |
| coding_profile | 3 | uimsbf |
| reserved | 4 | bslbf |
| channel_number | 8 | uimsbf |
| } | | |
| if (audio_codec_id == 2) { | | |
| nn_type | 3 | uimsbf |
| reserved | 1 | bslbf |
| content_type | 4 | uimsbf |
| content_type 0: channel_number_index (7 uimsbf), reserved (1 bslbf) | | |
| content_type 1: object_channel_number (7 uimsbf), reserved (1 bslbf) | | |
| content_type 2: channel_number_index (7), reserved (1), object_channel_number (7), reserved (1) | | |
| content_type 3: hoa_order (4 uimsbf), reserved (4 bslbf) | | |
| total_bitrate | 16 | uimsbf |
| } | | |
| resolution | 2 | uimsbf |
| reserved | 6 | bslbf |
| for (i = 0; i < N; i++) { addition_info[i] | 8 | bslbf } | | |
| } | | |

Table 5 content_type: 0 channel signal (coding_profile 0); 1 object signal (coding_profile 1, soundBedType 0); 2 channel+object mixed (coding_profile 1, soundBedType 1); 3 HOA (coding_profile 2); 4-15 reserved.

Table 6 total_bitrate (kbps): content_type 0 = bitrate of the channel signal's bitrate_index; 1 = bitrate_index_per_channel x (object_channel_number + 1); 2 = channel bitrate + object bitrate_index_per_channel x (object_channel_number + 1); 3 = HOA bitrate_index bitrate. Other field semantics: T/AI 109.3-2023 Annex A.
