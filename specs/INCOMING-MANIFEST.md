# Incoming Spec Manifest

Files downloaded 2026-06-27 for upcoming HLS/LL-HLS, codec, ID3, timed-metadata, MSE, QTFF, and container epics.

## Succeeded

| Filename | Standard | Source URL | Size | Format | Future Epic |
|---|---|---|---|---|---|
| `ietf_rfc8216_hls.txt` | IETF RFC 8216 — HTTP Live Streaming | https://www.rfc-editor.org/rfc/rfc8216.txt | 124 KB | TXT (PDF 404s at rfc-editor) | HLS / timed-metadata |
| `ietf_draft_pantos_hls_rfc8216bis.txt` | draft-pantos-hls-rfc8216bis-22 — HLS 2nd Edition (LL-HLS) | https://www.ietf.org/archive/id/draft-pantos-hls-rfc8216bis-22.txt | 270 KB | TXT | LL-HLS / timed-metadata |
| `itu_t_h264_avc.pdf` | ITU-T H.264 (08/2024) — Advanced Video Coding | https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.264-202408-I!!PDF-E&type=items | 14.7 MB | PDF (854 pages, `%PDF`) | codec / zenith-fMP4 |
| `itu_t_h265_hevc.pdf` | ITU-T H.265 (01/2026) — HEVC | https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.265-202601-I!!PDF-E&type=items | 11.7 MB | PDF (734 pages, `%PDF`) | codec / zenith-fMP4 |
| `id3v2.4.0_structure.txt` | ID3 v2.4.0 — Structure | https://id3.org/id3v2.4.0-structure | 95 KB | HTML (saved as .txt per instruction) | timed-metadata / ID3 |
| `id3v2.4.0_frames.txt` | ID3 v2.4.0 — Frames | https://id3.org/id3v2.4.0-frames | 217 KB | HTML (saved as .txt per instruction) | timed-metadata / ID3 |
| `apple_hls_timed_metadata.html` | Apple "Timed Metadata for HTTP Live Streaming" v1.2.1 | https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/HTTP_Live_Streaming_Metadata_Spec/Introduction/Introduction.html | 12 KB | HTML (single-page archive doc) | timed-metadata |
| `w3c_media_source_extensions.html` | W3C Media Source Extensions Level 2 | https://www.w3.org/TR/media-source-2/ | 904 KB | HTML | media-doctor / MSE |
| `apple_quicktime_file_format.pdf` | Apple QuickTime File Format (2001 classic spec) | https://developer.apple.com/standards/qtff-2001.pdf | 5.3 MB | PDF (`%PDF`) | zenith-fMP4 / ISOBMFF base |

**Total: ~33.3 MB across 9 files**

## Failed

| Filename | Standard | Attempted URL | Reason |
|---|---|---|---|
| `3gpp_ts_26244_3gp_file_format.pdf` | 3GPP TS 26.244 v19.0.0 — 3GP File Format | https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/19.00.00_60/ts_126244v190000p.pdf | Cloudflare WAF returns HTTP 403 for all automated curl fetches to etsi.org/deliver. Older ETSI versions (v12, v14) also 403. The 3GPP FTP archive (3gpp.org) only distributes .docx (Word) not PDF. **Workaround:** download manually from https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/19.00.00_60/ts_126244v190000p.pdf in a browser. |

## 2026-10-14 acquisition (docs/spec-acquisition-2026-10)

PDFs/HTML below live in the private submodule (`private/specs/`, markdown in `private/specs-md/`); RFC/W3C text lives here in `specs/`. All verified as real documents with a text layer (`pdfinfo` + `pdftotext`).

| Filename | Source URL | Licence / access | Notes |
|---|---|---|---|
| `etsi_en_300_468_v01.20.01_dvb_si.pdf` | https://www.etsi.org/deliver/etsi_en/300400_300499/300468/01.20.01_60/en_300468v012001p.pdf | ETSI free download (browser UA needed) | v1.20.1 (2026-09), 235 pp; newer than vendored v1.19.1 |
| `etsi_en_300_472_v01.04.01_dvb_teletext_system_b.pdf` | https://www.etsi.org/deliver/etsi_en/300400_300499/300472/01.04.01_60/en_300472v010401p.pdf | ETSI free | 13 pp |
| `etsi_ts_101_191_v01.04.01_dvb_mega_frame_sfn_sync.pdf` | https://www.etsi.org/deliver/etsi_ts/101100_101199/101191/01.04.01_60/ts_101191v010401p.pdf | ETSI free | 18 pp (latest = 1.4.1, 2004) |
| `etsi_ts_102_034_v02.01.01_dvb_mpeg2_ts_over_ip.pdf` | https://www.etsi.org/deliver/etsi_ts/102000_102099/102034/02.01.01_60/ts_102034v020101p.pdf | ETSI free | 331 pp |
| `etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf` | https://www.etsi.org/deliver/etsi_ts/103200_103299/103285/01.04.01_60/ts_103285v010401p.pdf | ETSI free | 149 pp |
| `etsi_ts_103_190_1_v01.04.01_ac4_part1.pdf` | https://www.etsi.org/deliver/etsi_ts/103100_103199/10319001/01.04.01_60/ts_10319001v010401p.pdf | ETSI free | 318 pp |
| `etsi_ts_126_244_v19.01.00_3gpp_3gp_file_format.pdf` | https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/19.01.00_60/ts_126244v190100p.pdf | ETSI free (resolves the 2026-06-27 "Failed" row above: works with browser UA + Referer) | 75 pp |
| `smpte_st291-1_2011_anc_packet_space_formatting.pdf` | https://pub.smpte.org/doc/st291-1/20110923-pub/st291-1-20110923-pub.zip | SMPTE open catalogue (free download) | 17 pp |
| `smpte_st336_2017_klv_data_encoding.pdf` | https://pub.smpte.org/doc/st336/20170721-pub/st336-20170721-pub.zip | SMPTE open catalogue | 36 pp |
| `smpte_st12-2_2014_timecode_in_anc.pdf` | https://pub.smpte.org/doc/st12-2/20140220-pub/st12-2-20140220-pub.zip | SMPTE open catalogue | 18 pp |
| `smpte_st12-3_2016_timecode_high_frame_rate.pdf` | https://pub.smpte.org/doc/st12-3/20160226-pub/st12-3-20160226-pub.zip | SMPTE open catalogue | 15 pp |
| `smpte_st2110-10_2022_system_timing.pdf` | https://pub.smpte.org/doc/st2110-10/20220328-pub/st2110-10-20220328-pub.zip | SMPTE open catalogue | 23 pp |
| `smpte_st2110-31_2022_aes3_transparent_transport.pdf` | https://pub.smpte.org/doc/st2110-31/20220624-pub/st2110-31-20220624-pub.zip | SMPTE open catalogue | 15 pp |
| `smpte_st2110-40_2023_anc_rtp.pdf` | https://pub.smpte.org/doc/st2110-40/20231231-pub/st2110-40-20231231-pub.zip | SMPTE open catalogue | 11 pp |
| `ms_sstr_smooth_streaming_protocol.pdf` | https://winprotocoldoc.z19.web.core.windows.net/MS-SSTR/%5bMS-SSTR%5d.pdf | Microsoft Open Specifications (free) | 65 pp |
| `dashif_cpix_v2.3.html` | https://dashif.org/docs/CPIX2.3/Cpix.html | DASH-IF guideline (free); UTF-16 source re-encoded to UTF-8 | |
| `rfc{3550,3551,4585,8285,6184,7798,8866,7826,9725,8794,9559,3533,6716,7845}_*.txt` | https://www.rfc-editor.org/rfc/rfc<N>.txt | IETF Trust (freely redistributable) | RTP, RTP A/V profile, RTCP feedback, RTP header ext, RTP H.264/HEVC, SDP, RTSP 2.0, WHIP, EBML, Matroska, Ogg, Opus, Ogg-Opus |
| `w3c_ttml2.html`, `w3c_imsc1.1.html`, `w3c_imsc1.2.html` | https://www.w3.org/TR/ttml2/ , /ttml-imsc1.1/ , /ttml-imsc1.2/ | W3C Document License (verbatim copies permitted) | for `ttml-subtitle` |

### Not obtainable freely / not acquired (2026-10-14)

- **ISO/IEC 14496-12/-14/-15/-3, 23009-1, 23001-7, 23000-19, 23008-3 (MPEG-H 3D Audio), 13818-x**: the ITTF "Publicly Available Standards" site is closed (page says deliverables moved to ISO/IEC webstores) and the ISO catalogue pages carry no free download; the ossrs spec index (v5/v6/v7) no longer lists ISO PDFs. Still paid (see `PAID-SPECS-TODO.md`).
- **ITU-T H.222.0 (04/2025)**: itu.int now demands a TIES login for the PDF (`choice_pub.asp`), vendored copy remains the 08/2023 edition.
- **CTA-608-E / CTA-708-E, SCTE 214-1/-3, SCTE 224**: registration-gated (CTA store "$0" checkout; SCTE account); not fetched, no account created.

## 2026-10-15 Chinese broadcast / AVS standards acquisition (docs/chinese-standards-acquisition)

All files below live in the private submodule (`private/specs/<file>`, Markdown in `private/specs-md/<stem>.md`, English working translations in `private/specs-md-en/`). Every PDF was downloaded into a fresh scratch directory and verified (`file` = PDF, `pdfinfo` page count, non-empty text layer via `pdftotext`; the three exceptions without a text layer were not vendored, see `docs/CHINESE-STANDARDS.md`). Licence/status for every row: **free public download with no login, captcha or payment**; the issuing body publishes the text for implementation use but grants no explicit redistribution licence, so the PDFs stay in the private repo (only the Markdown index/translations derived for implementation reference are in the submodule too, never in the public tree). Download date 2026-10-15. Sources: NRTA = `nrta.gov.cn` standards list (column 3715, 387 entries; each article page links `module/download/downfile.jsp`); UWA = `uhd-world-association.com/uwa-resources/`; AITISA = `aitisa.org.cn/detail?id=172` (AVS Workgroup / Zhongguancun audio-video industry alliance standards list).

| File | Standard | Source URL | Crates served |
|---|---|---|---|
| `nrta_gyt_161_2000_digital_audio_aux_data_in_ancillary_space.pdf` | GY/T 161-2000 digital audio and auxiliary data in DTV ancillary data space | https://www.nrta.gov.cn/art/2020/6/9/art_3715_51594.html | st291 |
| `nrta_gyt_163_2000_timecode_control_code_in_ancillary_data.pdf` | GY/T 163-2000 timecode and control code in DTV ancillary data space | https://www.nrta.gov.cn/art/2020/6/9/art_3715_51592.html | st12-1, st291 |
| `nrta_gyt_168_2001_broadcast_wave_format_bwf.pdf` | GY/T 168-2001 broadcast wave format (BWF) | https://www.nrta.gov.cn/art/2020/6/9/art_3715_51587.html | audio file formats (no crate yet) |
| `nrta_gyt_170_2001_cable_dtv_channel_coding_modulation.pdf` | GY/T 170-2001 cable DTV channel coding and modulation (DVB-C profile) | https://www.nrta.gov.cn/art/2020/6/9/art_3715_51585.html | context (dvb-bbframe family) |
| `nrta_gyt_212_2017_mpeg2_sd_encoder_decoder.pdf` | GY/T 212-2017 MPEG-2 SD encoder/decoder requirements | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51451.html | context |
| `nrta_gyt_214_2006_terrestrial_digital_audio_broadcast.pdf` | GY/T 214-2006 30-3000 MHz terrestrial digital audio broadcasting | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51450.html | context |
| `nrta_gyt_219_2006_time_signal_embedded_timecode.pdf` | GY/T 219-2006 time signal embedded timecode | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51446.html | st12-1 |
| `nrta_gyt_220_10_2008_cmmb_secure_broadcast.pdf` | GY/T 220.10-2008 CMMB part 10: secure broadcast | https://www.nrta.gov.cn/art/2020/5/22/art_3715_51338.html | context |
| `nrta_gyt_220_1_2006_cmmb_channel_frame_structure.pdf` | GY/T 220.1-2006 CMMB part 1: channel frame structure, coding, modulation | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51445.html | future CMMB parsers |
| `nrta_gyt_220_2_2006_cmmb_multiplexing.pdf` | GY/T 220.2-2006 CMMB part 2: multiplexing | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51444.html | mpeg-ts-like muxing context; future CMMB parser |
| `nrta_gyt_220_3_2007_cmmb_esg.pdf` | GY/T 220.3-2007 CMMB part 3: electronic service guide | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51443.html | dvb-si-like ESG context |
| `nrta_gyt_220_4_2007_cmmb_emergency_broadcast.pdf` | GY/T 220.4-2007 CMMB part 4: emergency broadcast | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51442.html | context |
| `nrta_gyt_220_5_2008_cmmb_data_broadcast.pdf` | GY/T 220.5-2008 CMMB part 5: data broadcast | https://www.nrta.gov.cn/art/2020/6/2/art_3715_51441.html | dvb-si data broadcast context |
| `nrta_gyt_220_6_2008_cmmb_conditional_access.pdf` | GY/T 220.6-2008 CMMB part 6: conditional access | https://www.nrta.gov.cn/art/2020/5/22/art_3715_51342.html | dvb-simulcrypt context |
| `nrta_gyt_220_7_2008_cmmb_receiver_terminal.pdf` | GY/T 220.7-2008 CMMB part 7: receiver terminal | https://www.nrta.gov.cn/art/2020/5/22/art_3715_51341.html | context |
| `nrta_gyt_220_8_2008_cmmb_multiplexer.pdf` | GY/T 220.8-2008 CMMB part 8: multiplexer | https://www.nrta.gov.cn/art/2020/5/22/art_3715_51340.html | context |
| `nrta_gyt_220_9_2008_cmmb_satellite_distribution.pdf` | GY/T 220.9-2008 CMMB part 9: satellite distribution channel | https://www.nrta.gov.cn/art/2020/5/22/art_3715_51339.html | context |
| `nrta_gyt_226_2007_dtv_multiplexer.pdf` | GY/T 226-2007 DTV multiplexer requirements and measurement | https://www.nrta.gov.cn/art/2020/5/11/art_3715_51125.html | mpeg-ts, dvb-conformance |
| `nrta_gyt_246_2020_iptv_drm.pdf` | GY/T 246-2020 content-distribution DRM: IPTV DRM system | https://www.nrta.gov.cn/art/2020/11/12/art_3715_53799.html | transmux CENC / cpix context |
| `nrta_gyt_255_2012_downloadable_cas.pdf` | GY/T 255-2012 downloadable CAS | https://www.nrta.gov.cn/art/2020/5/11/art_3715_51107.html | dvb-simulcrypt / CAS context |
| `nrta_gyt_255_2024_downloadable_cas.pdf` | GY/T 255-2024 downloadable CAS (revision) | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69952.html | dvb-simulcrypt / CAS context |
| `nrta_gyt_257_1_2012_avs_plus_video.pdf` | GY/T 257.1-2012 AVS+ video | https://www.nrta.gov.cn/art/2020/5/11/art_3715_51106.html | mpeg-ts/dvb-si (stream types), transmux |
| `nrta_gyt_262_2012_programme_loudness_true_peak_algorithm.pdf` | GY/T 262-2012 programme loudness / true-peak measurement algorithm (BS.1770-2) | https://www.nrta.gov.cn/art/2020/5/8/art_3715_51071.html | broadcast-loudness |
| `nrta_gyt_263_2012_loudness_true_peak_meter_requirements.pdf` | GY/T 263-2012 loudness and true-peak meter requirements (BS.1771) | https://www.nrta.gov.cn/art/2020/5/8/art_3715_51055.html | broadcast-loudness |
| `nrta_gyt_268_1_2013_cdr_fm_band_digital_audio_broadcast_frame.pdf` | GY/T 268.1-2013 CDR (FM-band digital audio broadcast) part 1: frame structure | https://www.nrta.gov.cn/art/2020/5/8/art_3715_51049.html | future CDR crate |
| `nrta_gyt_268_2_2013_cdr_multiplexing.pdf` | GY/T 268.2-2013 CDR part 2: multiplexing | https://www.nrta.gov.cn/art/2020/5/8/art_3715_51048.html | future CDR crate |
| `nrta_gyt_270_2013_dtv_closed_caption_system.pdf` | GY/T 270-2013 DTV closed caption system (CEA-708 style, China profile) | https://www.nrta.gov.cn/art/2020/5/8/art_3715_51046.html | cc-data, caption-convert, dvb-si (caption_service_descriptor 0x86) |
| `nrta_gyt_281_2014_audio_extended_file_format_mbwf_rf64.pdf` | GY/T 281-2014 MBWF / RF64 extended audio file format | https://www.nrta.gov.cn/art/2020/4/29/art_3715_50939.html | audio file formats (no crate yet) |
| `nrta_gyt_282_2014_dtv_programme_loudness_true_peak_requirements.pdf` | GY/T 282-2014 DTV programme loudness (-24 LKFS) and true-peak (-2 dBTP) requirements | https://www.nrta.gov.cn/art/2020/4/29/art_3715_50938.html | broadcast-loudness |
| `nrta_gyt_286_2014_dtv_codec_contribution_distribution_snG.pdf` | GY/T 286-2014 DTV codec requirements for contribution/distribution/SNG | https://www.nrta.gov.cn/art/2020/4/29/art_3715_50934.html | context |
| `nrta_gyt_287_2024_dtv_codec_emission_secondary_distribution.pdf` | GY/T 287-2024 DTV codec requirements for emission and secondary distribution | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69953.html | context |
| `nrta_gyt_299_1_2016_avs2_video.pdf` | GY/T 299.1-2016 AVS2 video | https://www.nrta.gov.cn/art/2020/4/28/art_3715_50882.html | transmux (av2c), dvb-si (AVS2_video_descriptor, tag 64) |
| `nrta_gyt_301_2016_video_dialogue_subtitle_data_format.pdf` | GY/T 301-2016 video programme dialogue-subtitle data format | https://www.nrta.gov.cn/art/2020/4/28/art_3715_50880.html | caption-convert |
| `nrta_gyt_304_2016_high_performance_streaming_audio_ip_interoperability.pdf` | GY/T 304-2016 high-performance streaming audio over IP interoperability (AES67 equivalent) | https://www.nrta.gov.cn/art/2020/4/28/art_3715_50876.html | rtp-packet |
| `nrta_gyt_307_2017_uhdtv_production_exchange_parameters.pdf` | GY/T 307-2017 UHDTV production and exchange parameter values | https://www.nrta.gov.cn/art/2020/4/28/art_3715_50873.html | context |
| `nrta_gyt_308_2017_unidirectional_downloadable_cas.pdf` | GY/T 308-2017 unidirectional downloadable CAS | https://www.nrta.gov.cn/art/2020/4/28/art_3715_50872.html | dvb-simulcrypt / CAS context |
| `nrta_gyt_322_1_2019_oca_open_control_architecture_framework.pdf` | GY/T 322.1-2019 OCA (AES70) framework | https://www.nrta.gov.cn/art/2020/4/24/art_3715_50827.html | context |
| `nrta_gyt_322_2_2019_oca_class_structure.pdf` | GY/T 322.2-2019 OCA class structure | https://www.nrta.gov.cn/art/2020/4/24/art_3715_50826.html | context |
| `nrta_gyt_322_3_2019_oca_tcp_ip_network_adaptation.pdf` | GY/T 322.3-2019 OCA TCP/IP adaptation | https://www.nrta.gov.cn/art/2020/4/24/art_3715_50825.html | context |
| `nrta_gyt_333_2020_cable_dtv_drm.pdf` | GY/T 333-2020 content-distribution DRM: cable DTV DRM system | https://www.nrta.gov.cn/art/2020/11/12/art_3715_53795.html | transmux CENC / cpix context |
| `nrta_gyt_338_2020_dtv_satellite_channel_coding_modulation.pdf` | GY/T 338-2020 DTV satellite channel coding and modulation | https://www.nrta.gov.cn/art/2021/1/8/art_3715_54680.html | dvb-bbframe (DVB-S2 profile) context |
| `nrta_gyt_341_2020_ip_based_terrestrial_dtv_sfn.pdf` | GY/T 341-2020 IP-based terrestrial DTV single frequency network | https://www.nrta.gov.cn/art/2021/1/8/art_3715_54685.html | dvb-t2mi / dvb-stream context |
| `nrta_gyt_347_1_2021_uhdtv_serial_digital_interface_multilink_10g_10bit.pdf` | GY/T 347.1-2021 UHDTV serial digital interface, part 1 (multilink 10G, 10-bit) | https://www.nrta.gov.cn/art/2021/4/2/art_3715_55658.html | st291 |
| `nrta_gyt_347_2_2021_uhdtv_serial_digital_interface_multilink_10g_12bit.pdf` | GY/T 347.2-2021 UHDTV serial digital interface, part 2 (multilink 10G, 12-bit) | https://www.nrta.gov.cn/art/2021/4/2/art_3715_55659.html | st291 |
| `nrta_gyt_347_3_2021_uhdtv_serial_digital_interface_single_multilink_6g_12g_24g.pdf` | GY/T 347.3-2021 UHDTV serial digital interface, part 3 (6G/12G/24G) | https://www.nrta.gov.cn/art/2021/4/2/art_3715_55660.html | st291 |
| `nrta_gyt_348_2021_precise_time_synchronization_protocol_pro_av.pdf` | GY/T 348-2021 precise time synchronisation protocol for pro A/V (PTP profile) | https://www.nrta.gov.cn/art/2021/4/2/art_3715_55661.html | context |
| `nrta_gyt_358_2022_hdr_display_adaptation_metadata_en.pdf` | GY/T 358-2022 HDR display adaptation metadata (English edition hosted by UWA) | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA_2-GYT-358-2022-Technical-Requirements-for-Display-Adaptation-Metadataof-High-Dynamic-Range-Television-Systems0913.pdf.pdf | transmux (SEI metadata), dvb-si |
| `nrta_gyt_360_2022_content_identification_tagging.pdf` | GY/T 360-2022 programme content identification tags | https://www.nrta.gov.cn/art/2022/9/29/art_3715_61975.html | timed-metadata context |
| `nrta_gyt_365_2023_4k_uhdtv_programme_file_format.pdf` | GY/T 365-2023 4K UHDTV programme file format (MXF mapping) | https://www.nrta.gov.cn/art/2023/3/9/art_3715_63625.html | st377-1 |
| `nrta_gyt_368_2023_avs3_video_coding.pdf` | GY/T 368-2023 AVS3 video (advanced efficient video coding) | https://www.nrta.gov.cn/art/2023/8/30/art_3715_65407.html | transmux (AVS3 sequence header, av3c), dvb-si (AVS3_video_descriptor fields) |
| `nrta_gyt_372_2023_aoip_gateway_requirements.pdf` | GY/T 372-2023 AoIP gateway requirements | https://www.nrta.gov.cn/art/2023/11/17/art_3715_66149.html | rtp-packet (AES67 family) |
| `nrta_gyt_377_2023_online_av_loudness_requirements.pdf` | GY/T 377-2023 online A/V programme loudness (-15/-24 LKFS, -1 dBTP) | https://www.nrta.gov.cn/art/2023/9/14/art_3715_65554.html | broadcast-loudness |
| `nrta_gyt_379_2023_dtv_audio_loudness_monitoring.pdf` | GY/T 379-2023 DTV audio loudness monitoring procedure | https://www.nrta.gov.cn/art/2023/9/14/art_3715_65556.html | broadcast-loudness, compliance-probe |
| `nrta_gyt_398_1_2024_light_compression_video_uhd.pdf` | GY/T 398.1-2024 light-compression video coding, part 1: UHD | https://www.nrta.gov.cn/art/2024/1/8/art_3715_67802.html | transmux (future codec id) |
| `nrta_gyt_404_2024_audio_definition_model.pdf` | GY/T 404-2024 Audio definition model (ADM, Chinese counterpart of ITU-R BS.2076) | https://www.nrta.gov.cn/art/2024/8/22/art_3715_68565.html | future ADM/metadata work; transmux audio metadata |
| `nrta_gyt_414_2024_ip_production_uncompressed_video_streams.pdf` | GY/T 414-2024 IP production system: uncompressed video streams (ST 2110-20 equivalent) | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69947.html | rtp-packet, st2022 family |
| `nrta_gyt_415_2024_ip_production_system_timing_definitions.pdf` | GY/T 415-2024 IP production system: system timing and definitions (ST 2110-10 equivalent) | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69948.html | rtp-packet |
| `nrta_gyt_416_2024_ip_production_video_traffic_shaping_timing.pdf` | GY/T 416-2024 IP production system: video traffic shaping and delivery timing (ST 2110-21 equivalent) | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69949.html | rtp-packet |
| `nrta_gyt_417_2024_ip_realtime_stream_seamless_protection_switching.pdf` | GY/T 417-2024 seamless protection switching of IP real-time streams (ST 2022-7 equivalent) | https://www.nrta.gov.cn/art/2025/1/6/art_3715_69950.html | rist-runtime, st2022 |
| `nrta_gyt_419_2025_hdr_video_end_to_end.pdf` | GY/T 419-2025 HDR end-to-end (HDR Vivid metadata generation/coding/transport) | https://www.nrta.gov.cn/art/2025/4/14/art_3715_70464.html | transmux (SEI/metadata carriage), dvb-si (HDR component types) |
| `nrta_gyt_420_2025_uhd_av_stream_encapsulation_network_transmission.pdf` | GY/T 420-2025 UHD A/V stream encapsulation for network transmission (TS, SMT, ISOBMFF, FLV, DASH) | https://www.nrta.gov.cn/art/2025/4/14/art_3715_70465.html | mpeg-ts (StreamType 0xD2/0xD4/0xD5), dvb-si (descriptors 64/209/210, registration AVSV/AVSA), mpeg-pes (extended stream_id), transmux (av3c/lavc/av3a/dca3 boxes), broadcast-hls, rtmp-runtime |
| `nrta_gyt_422_2025_tv_multi_audio_transmission.pdf` | GY/T 422-2025 TV programme multi-audio transmission (audio_preselection_descriptor) | https://www.nrta.gov.cn/art/2025/8/1/art_3715_71221.html | dvb-si (audio_preselection descriptor) |
| `nrta_gyt_427_2026_uhd_mobile_distribution_format.pdf` | GY/T 427-2026 UHD distribution format for mobile terminals | https://www.nrta.gov.cn/art/2026/5/14/art_3715_73293.html | context only |
| `nrta_gyz_175_2001_dtv_conditional_access_system.pdf` | GY/Z 175-2001 DTV conditional access system specification (SimulCrypt-style) | https://www.nrta.gov.cn/art/2020/6/9/art_3715_51581.html | dvb-simulcrypt, dvb-csa, dvb-si (CA descriptors), dvb-ci |
| `uwa_tuwa_005_1_2022_hdr_vivid_metadata_tone_mapping.pdf` | T/UWA 005.1-2022 HDR Vivid technical standard: metadata and tone mapping (English, v1.0, 2022-02-10) | https://www.uhd-world-association.com/wp-content/uploads/2025/03/TUWA-005.1-2022-HDR-Video-Technology-Part-1-Metadata-and-Tone-Mapping.pdf | dvb-si (T/UWA 005 DMI, TS 101 154 clause 5.14.4.4.3.4.6), transmux (SEI/T.35 carriage) |
| `uwa_tuwa_005_2_1_2026_hdr_system_integration_guide.pdf` | T/UWA 005.2-1-2026 HDR Vivid application guide to system integration (ITU-T T.35 payload table 2; cited by TS 101 154 v2.10.1 [62]) | https://www.uhd-world-association.com/wp-content/uploads/2026/09/TUWA-005.2-1-2026-HDR-Video-Technology-Part-2-1-Application-Guide-to-System-Integration-0924clean.pdf | dvb-si / transmux (HDR Vivid SEI) |
| `uwa_tuwa_005_2_2_2022_hdr_post_production_guide.pdf` | T/UWA 005.2-2-2022 HDR Vivid application guide: post production | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA-7-TUWA-005.2-2-2022-HDR-Video-Technology-Part-2-2-Application-Guide-Post-Production-20230906.pdf | context |
| `uwa_tuwa_005_3_3_2022_hdr_player_device_requirements.pdf` | T/UWA 005.3-3-2022 HDR Vivid player-device requirements and test methods | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA-10-TUWA-005.3-3-2022-HDR-Video-Technology-Part-3-3-Technical-Requirements-and-Test-Methods-Player-Device.pdf | context |
| `uwa_tuwa_005_3_4_2022_hdr_player_software_requirements.pdf` | T/UWA 005.3-4-2022 HDR Vivid player-software requirements and test methods | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA-11-TUWA-005.3-4-2022-HDR-Video-Technology-Part-3-4-Technical-Requirements-and-Test-Methods-Player-Software.pdf | context |
| `uwa_tuwa_005_3_5_2022_hdr_live_encoder_requirements.pdf` | T/UWA 005.3-5-2022 HDR Vivid live encoder requirements and test methods | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA-12-TUWA-005.3-5-2022-HDR-Video-Technology-Part-3-5-Technical-Requirements-and-Test-Methods-Live-Encoder-Device-20230911.pdf | context |
| `uwa_tuwa_008_2022_4k_uhd_content_exchange_parameters.pdf` | T/UWA 008-2022 parameters for 4K UHD content exchange | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA-14-TUWA-008-2022-Parameters-Specifications-for-4K-Ultra-HD-ContentExchange.pdf | context |
| `uwa_tuwa_009_1_2023_3d_audio_coding_transmission_presentation.pdf` | T/UWA 009.1-2023 3D Audio technology specification part 1: coding, transmission and presentation (English, Audio Vivid; 374 pp) | https://www.uhd-world-association.com/wp-content/uploads/2024/08/3DAudio-Technology-Specification-Part1.pdf | transmux (av3a/dca3), dvb-si (AVS3_audio_descriptor), future Audio Vivid crate |
| `uwa_tuwa_012_2_2023_bcqp_uhd_system_ts_encapsulation.pdf` | T/UWA 012.2-2023 "Bai Cheng Qian Ping" UHD A/V coding: system (TS stream_id/stream_type/AVS3 descriptor, extended PES TimeStamp, TS over UDP/RTP) | https://www.uhd-world-association.com/wp-content/uploads/2024/10/19-TUWA-012.2%E2%80%942023Coding-of-UHD-video-and-audio-broadcasting-system-system.pdf | mpeg-ts, dvb-si, mpeg-pes, dvb-stream |
| `uwa_tuwa_012_3_2022_bcqp_uhd_video_coding.pdf` | T/UWA 012.3-2022 UHD A/V coding: video | https://www.uhd-world-association.com/wp-content/uploads/2024/10/20-TUWA-012.3%E2%80%942022Coding-of-UHD-video-and-audio-broadcasting-system-video.pdf | transmux |
| `uwa_tuwa_012_4_2023_bcqp_professional_decoder.pdf` | T/UWA 012.4-2023 UHD professional decoder technical requirements | https://www.uhd-world-association.com/wp-content/uploads/2024/10/21-TUWA-012.4%E2%80%942023Technical-requirements-of-professional-decoder.pdf | context |
| `uwa_tuwa_012_6_2023_bcqp_network_transmission.pdf` | T/UWA 012.6-2023 UHD A/V: network transmission technical requirements | https://www.uhd-world-association.com/wp-content/uploads/2024/10/23-TUWA-012.6%E2%80%942023Technical-requirements-of-network-transmission.pdf | dvb-stream, multimux |
| `uwa_w00001_2022_audio_vivid_white_paper.pdf` | UWA W00001-2022 Audio Vivid technical white paper (informative) | https://www.uhd-world-association.com/wp-content/uploads/2024/10/30-W00001-202208-Audio-Vivid-Technical-White-Paper-.pdf | context |
| `uwa_w00002_2022_hdr_vivid_white_paper.pdf` | UWA W00002-2022 HDR Vivid technical white paper (informative) | https://www.uhd-world-association.com/wp-content/uploads/2024/10/31-W00002-202212-HDR-Vivid-Technical-White-Paper-.pdf | context |
| `t_ai_109_6_2022_avs3_intelligent_media_format.pdf` | T/AI 109.6-2022 Intelligent media format (AVS3 video system layer: TS stream_type 0xD4, stream_id 0xFD ext 0x41/0x42, AVS3_video_descriptor tag 209, data_stream_alignment_descriptor; CMAF brand ca3v). Cited by TS 101 154 v2.10.1 [56] | http://www.aitisa.org.cn/ueditor/php/upload/file/20230913/1694581869.pdf | mpeg-ts, dvb-si, mpeg-pes, transmux |
| `t_ai_109_2_2021_avs3_video_coding.pdf` | T/AI 109.2-2021 AVS3 video (Chinese; file re-issued 2024-11-22 on the site). Cited by TS 101 154 v2.10.1 [55] | http://www.aitisa.org.cn/ueditor/php/upload/file/20241122/1732258705.pdf | transmux, dvb-si |
| `t_ai_109_3_2023_avs3_immersive_audio_cn.pdf` | T/AI 109.3-2023 immersive audio (Chinese authoritative text; AATF/AASF in Annex A). Cited by TS 101 154 v2.10.1 [58] | http://www.aitisa.org.cn/ueditor/php/upload/file/20230913/1694581904.pdf | transmux, dvb-si, future AVS3-P3 crate |
| `t_ai_114_2024_intelligent_media_transport_smt.pdf` | T/AI 114-2024 high-efficiency multimedia coding part 6: intelligent media transport (SMT revision; the 2021 first edition was fetched, found superseded and not vendored) | http://www.aitisa.org.cn/ueditor/php/upload/file/20241015/1728973215.pdf | transmux (SMT), future SMT parser |
| `nrta_gyt_363_2023_3d_audio_coding_rendering_en.pdf` | GY/T 363-2023 3D audio coding and rendering, official English edition hosted by UWA (452 pp); complements the Chinese PDF vendored separately | https://www.uhd-world-association.com/wp-content/uploads/2024/10/UWA_3-GYT-363-2023-3D-Audio-Coding-and-Rendering.pdf | transmux, dvb-si, future Audio Vivid crate |

Vendored by a parallel agent in the same wave (not re-downloaded here): `nrta_gyt_363_2023_3d_audio_coding_rendering.pdf` and `nrta_gyt_316_2018_advanced_sound_system.pdf` (NRTA), `t_ai_109_3_2023_avs3_immersive_audio_en.pdf` (English edition; byte-different from the AITISA English copy only in its equation text layer) and `t_ai_109_7_2024_avs3_audio_encapsulation_transport.pdf` (byte-identical to the AITISA download).


### Not obtainable freely / not acquired (2026-10-15, Chinese standards)

See `docs/CHINESE-STANDARDS.md` section 4: SAC GB/T texts (DRA GB/T 22726, DTMB GB/T 20600 / DTMB-A, GB/T 28161 SI, GB/T 17975.1, GB/T 46269.1) are online-preview only; ttbz.org.cn is behind a WAF slider captcha (not bypassed); avs.org.cn downloads are member-gated; GY/T 106, 361, 362 are image-only PDFs; GY/Z 174 / GY/T 229 are not published on NRTA's list.
