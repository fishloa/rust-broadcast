## Table 2-45 — Program and program element descriptors
_Rec. ITU-T H.222.0 (06/2021) | ISO/IEC 13818-1:2021 §2.6.1, Table 2-45; PDF pp.77. The TS/PS columns mark applicability to transport stream / program stream ('X' = applicable)._

| descriptor_tag | TS | PS | Identification |
|---|---|---|---|
| 0 | n/a | n/a | Reserved |
| 1 | n/a | X | Forbidden |
| 2 | X | X | video_stream_descriptor |
| 3 | X | X | audio_stream_descriptor |
| 4 | X | X | hierarchy_descriptor |
| 5 | X | X | registration_descriptor |
| 6 | X | X | data_stream_alignment_descriptor |
| 7 | X | X | target_background_grid_descriptor |
| 8 | X | X | video_window_descriptor |
| 9 | X | X | CA_descriptor |
| 10 | X | X | ISO_639_language_descriptor |
| 11 | X | X | system_clock_descriptor |
| 12 | X | X | multiplex_buffer_utilization_descriptor |
| 13 | X | X | copyright_descriptor |
| 14 | X |  | maximum_bitrate_descriptor |
| 15 | X | X | private_data_indicator_descriptor |
| 16 | X | X | smoothing_buffer_descriptor |
| 17 | X |  | STD_descriptor |
| 18 | X | X | IBP_descriptor |
| 19..26 | X |  | Defined in ISO/IEC 13818-6 |
| 27 | X | X | MPEG-4_video_descriptor |
| 28 | X | X | MPEG-4_audio_descriptor |
| 29 | X | X | IOD_descriptor |
| 30 | X | X | SL_descriptor |
| 31 | X | X | FMC_descriptor |
| 32 | X | X | external_ES_ID_descriptor |
| 33 | X | X | MuxCode_descriptor |
| 34 | X | X | FmxBufferSize_descriptor |
| 35 | X |  | multiplexBuffer_descriptor |
| 36 | X | X | content_labeling_descriptor |
| 37 | X | X | metadata_pointer_descriptor |
| 38 | X | X | metadata_descriptor |
| 39 | X | X | metadata_STD_descriptor |
| 40 | X | X | AVC video descriptor |
| 41 | X | X | IPMP_descriptor (defined in ISO/IEC 13818-11, MPEG-2 IPMP) |
| 42 | X | X | AVC timing and HRD descriptor |
| 43 | X | X | MPEG-2_AAC_audio_descriptor |
| 44 | X | X | FlexMuxTiming_descriptor |
| 45 | X | X | MPEG-4_text_descriptor |
| 46 | X | X | MPEG-4_audio_extension_descriptor |
| 47 | X | X | Auxiliary_video_stream_descriptor |
| 48 | X | X | SVC extension descriptor |
| 49 | X | X | MVC extension descriptor |
| 50 | X | n/a | J2K video descriptor |
| 51 | X | X | MVC operation point descriptor |
| 52 | X | X | MPEG2_stereoscopic_video_format_descriptor |
| 53 | X | X | Stereoscopic_program_info_descriptor |
| 54 | X | X | Stereoscopic_video_info_descriptor |
| 55 | X | n/a | Transport_profile_descriptor |
| 56 | X | n/a | HEVC video descriptor |
| 57 | X | n/a | VVC video descriptor |
| 58 | X | n/a | EVC video descriptor |
| 59..62 | n/a | n/a | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 Reserved |
| 63 | X | X | Extension_descriptor |
| 64..255 | n/a | n/a | User Private |
