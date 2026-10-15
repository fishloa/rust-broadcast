# Vendored specification PDFs

Canonical source-of-truth standards for the workspace crates. The structured
markdown under each crate's `docs/` is transcribed from these, and **that
markdown is the public, reviewable artefact** the parsers are checked against.

> **The PDFs themselves are now private.** Every spec PDF lives in the private
> companion repo **`rust-broadcast-specs`** (checked out as a sibling at
> `../rust-broadcast-specs`), NOT in this repo. `specs/*.pdf` is gitignored
> here. This keeps redistribution-restricted standards (ISO/IEC, ITU-T, and —
> conservatively — even the freely-downloadable ETSI/DVB/SCTE deliverables) out
> of the public tree, while the derived syntax markdown stays public.
>
> To regenerate a `docs/` transcription, clone `rust-broadcast-specs` next to
> this repo (`git clone …/rust-broadcast-specs ../rust-broadcast-specs`) so the
> PDFs resolve at `specs/` (symlink or copy as needed). CI does not need them —
> it builds from the committed markdown.

Freely-redistributable **text** specs (IETF RFCs incl. RTP/RTCP/SDP/RTSP2/WHIP/EBML/Matroska/Ogg/Opus, W3C TTML2/IMSC, HLS drafts, ID3) remain
committed here as `.txt`/`.html`. The table below documents the PDF set that
now lives in `rust-broadcast-specs`.

| File | Standard | Used by |
|---|---|---|
| `etsi_en_300_468_v01.19.01_dvb_si.pdf` | EN 300 468 v1.19.1 — DVB Service Information | `dvb-si` (all SI tables, incl. SAT §5.2.11) |
| `etsi_en_301_192_v01.07.01_dvb_databcast.pdf` | EN 301 192 v1.7.1 — DVB data broadcasting | `dvb-si` (INT, table_id 0x4C) |
| `etsi_ts_102_006_v01.07.01_dvb_ssu.pdf` | TS 102 006 v1.7.1 — System Software Update | `dvb-si` (UNT, table_id 0x4B) |
| `etsi_ts_102_323_v01.04.01_dvb_tvanytime.pdf` | TS 102 323 v1.4.1 — TV-Anytime carriage | `dvb-si` (RCT 0x76, CIT 0x77, RNT 0x79, container 0x75) |
| `etsi_ts_102_809_v01.03.01_dvb_app_signalling.pdf` | TS 102 809 v1.3.1 — Signalling and carriage of interactive applications | `dvb-si` (AIT 0x74, protection message 0x7B) |
| `etsi_ts_102_772_v01.01.01_dvb_mpe_ifec.pdf` | TS 102 772 v1.1.1 — MPE inter-burst FEC | `dvb-si` (MPE-IFEC section 0x7A) |
| `etsi_en_303_560_v01.01.01_dvb_ttml_fonts.pdf` | EN 303 560 v1.1.1 — TTML subtitling systems | `dvb-si` (downloadable font info 0x7C) |
| `etsi_ts_102_727_v01.01.01_dvb_mhp.pdf` | TS 102 727 v1.1.1 — Multimedia Home Platform 1.2.2 | `dvb-si` (XAIT_location descriptor 0x7D, §10.17.6) |
| `etsi_tr_101_202_v01.02.01_dvb_data_broadcasting_guidelines.pdf` | TR 101 202 v1.2.1 — Data broadcasting implementation guidelines | `dvb-si` (`carousel` module — DVB profile semantics; the message byte-syntax itself is ISO/IEC 13818-6, see `dvb-si/docs/iso_13818_6_carousel.md`) |
| `etsi_en_302_307_1_v01.04.01_dvb_s2.pdf` | EN 302 307-1 — DVB-S2 | `dvb-bbframe` |
| `etsi_en_302_307_2_v01.04.01_dvb_s2x.pdf` | EN 302 307-2 — DVB-S2X | `dvb-bbframe` |
| `etsi_en_302_755_v01.04.01_dvb_t2.pdf` | EN 302 755 v1.4.1 — DVB-T2 | `dvb-bbframe`, `dvb-t2mi` |
| `etsi_ts_102_773_v01.04.01_dvb_t2_modulator_interface.pdf` | TS 102 773 v1.4.1 — T2-MI | `dvb-t2mi` |
| `etsi_tr_101_290_v01.04.01_dvb_measurement.pdf` | TR 101 290 v1.4.1 — Measurement guidelines for DVB systems | planned `dvb-conformance` (#57); tables in `dvb-si/docs/tr_101_290.md` |
| `etsi_tr_101_211_v01.09.01_dvb_si_guidelines.pdf` | TR 101 211 v1.9.1 — SI implementation guidelines | `dvb-si` (SI repetition rates — packetiser/SiMux #56, conformance #57) |
| `ansi_scte_35_2023r1_dpi_cueing.pdf` | ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message | `scte35-splice` (#58); tables in `scte35-splice/docs/tables/scte_35/` |
| `dvb_a001r18_draft_ts_101_154_v02.07.01_av_coding.pdf` | DVB A001r18 (draft TS 101 154 v2.7.1) — AV coding in broadcast/broadband | `dvb-si` (component/AC-4/audio-preselection semantics, #53) |
| `etsi_ts_103_190_2_v01.03.01_ac4.pdf` | TS 103 190-2 v1.3.1 — AC-4 immersive/personalized audio | `dvb-si` (AC-4 descriptor `ac4_toc` structure §6.2.1 — deferred typing, #102) |
| `etsi_en_300_468_v01.20.01_dvb_si.pdf` | EN 300 468 v1.20.1 (2026-09) — DVB SI, newer edition than the v1.19.1 above | `dvb-si` (diff against v1.19.1 for new descriptors/tables) |
| `etsi_en_300_472_v01.04.01_dvb_teletext_system_b.pdf` | EN 300 472 v1.4.1 — ITU-R System B Teletext in DVB bitstreams | `dvb-vbi`, teletext carriage |
| `etsi_ts_101_191_v01.04.01_dvb_mega_frame_sfn_sync.pdf` | TS 101 191 v1.4.1 — DVB mega-frame for SFN synchronization | `dvb-t2mi` / SFN context |
| `etsi_ts_102_034_v02.01.01_dvb_mpeg2_ts_over_ip.pdf` | TS 102 034 v2.1.1 — MPEG-2 TS based DVB services over IP | `dvb-stream`, `multimux` (RTP/UDP TS) |
| `etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf` | TS 103 285 v1.4.1 — MPEG-DASH profile for DVB services over IP | `transmux` / `multimux` DASH |
| `etsi_ts_103_190_1_v01.04.01_ac4_part1.pdf` | TS 103 190-1 v1.4.1 — AC-4 part 1 (channel-based coding) | `dvb-si` AC-4 descriptor |
| `etsi_ts_126_244_v19.01.00_3gpp_3gp_file_format.pdf` | 3GPP TS 26.244 v19.1.0 — 3GP file format | `transmux` (3GP/ISOBMFF brands) |
| `smpte_st291-1_2011_anc_packet_space_formatting.pdf` | SMPTE ST 291-1:2011 — ANC packet and space formatting | `st291` |
| `smpte_st336_2017_klv_data_encoding.pdf` | SMPTE ST 336:2017 — KLV data encoding protocol | `st377-1` (KLV framing) |
| `smpte_st12-2_2014_timecode_in_anc.pdf` | SMPTE ST 12-2:2014 — Timecode transmission in ANC | `st12-1`, `st291` |
| `smpte_st12-3_2016_timecode_high_frame_rate.pdf` | SMPTE ST 12-3:2016 — Timecode for high frame rate signals | `st12-1` |
| `smpte_st2110-10_2022_system_timing.pdf` | SMPTE ST 2110-10:2022 — system and timing | ST 2110 family (`cpix/`/`st2110/` docs) |
| `smpte_st2110-31_2022_aes3_transparent_transport.pdf` | SMPTE ST 2110-31:2022 — AES3 transparent transport | `st337` over RTP |
| `smpte_st2110-40_2023_anc_rtp.pdf` | SMPTE ST 2110-40:2023 — ancillary data over RTP | `st291` (`rtp` feature) |
| `ms_sstr_smooth_streaming_protocol.pdf` | MS-SSTR — Smooth Streaming Protocol (Microsoft Open Specifications) | `transmux` Smooth |
| `dashif_cpix_v2.3.html` | DASH-IF CPIX v2.3 — Content Protection Information Exchange Format | `cpix/` docs-only, `transmux` CENC |

## Chinese broadcast / AVS standards (private submodule, 2026-10)

About 85 free-to-download Chinese broadcast and AVS standards (NRTA `GY/T`, AVS Workgroup / AITISA `T/AI`, UHD World Association `T/UWA`) were vendored into the private submodule: PDFs in `private/specs/`, textlayer Markdown in `private/specs-md/`, unofficial English working translations of the implementation-relevant clauses in `private/specs-md-en/`. Every file with its source URL, free/licence status and the crates it serves is in [`INCOMING-MANIFEST.md`](INCOMING-MANIFEST.md); the ranked index, edition tracking, blocked items and follow-up opportunities are in [`../docs/CHINESE-STANDARDS.md`](../docs/CHINESE-STANDARDS.md). The ones to read first:

| File (in `private/specs/`) | Standard | Used by |
|---|---|---|
| `t_ai_109_7_2024_avs3_audio_encapsulation_transport.pdf` | T/AI 109.7-2024 — AVS3 audio encapsulation and transport (`AVS3_audio_descriptor`, stream_type 0xD5, RTP) | `dvb-si`, `mpeg-ts`, `mpeg-pes`, `transmux` |
| `t_ai_109_6_2022_avs3_intelligent_media_format.pdf` | T/AI 109.6-2022 — AVS3 video system layer (`AVS3_video_descriptor`, stream_type 0xD4, stream_id 0xFD/0x41) | `dvb-si`, `mpeg-ts`, `mpeg-pes`, `transmux` |
| `nrta_gyt_420_2025_uhd_av_stream_encapsulation_network_transmission.pdf` | GY/T 420-2025 — UHD A/V encapsulation (TS, SMT, ISOBMFF, FLV, DASH) | `dvb-si`, `transmux`, `broadcast-hls`, `rtmp-runtime` |
| `uwa_tuwa_012_2_2023_bcqp_uhd_system_ts_encapsulation.pdf` | T/UWA 012.2-2023 — UHD system layer (earlier AVS3 descriptor layout, extended PES TimeStamp) | `dvb-si`, `mpeg-pes` |
| `t_ai_109_2_2021_avs3_video_coding.pdf`, `nrta_gyt_368_2023_avs3_video_coding.pdf` | AVS3 video (sequence header, profiles/levels) | `transmux`, `dvb-si` |
| `t_ai_109_3_2023_avs3_immersive_audio_cn.pdf`, `nrta_gyt_363_2023_3d_audio_coding_rendering_en.pdf` | AVS3 audio / GY/T 363-2023 (AATF/AASF) | `transmux`, future AVS3-P3 crate |
| `uwa_tuwa_005_1_2022_hdr_vivid_metadata_tone_mapping.pdf`, `uwa_tuwa_005_2_1_2026_hdr_system_integration_guide.pdf` | T/UWA 005.1 / 005.2-1 — HDR Vivid metadata (EN 300 468 "T/UWA 005 DMI") | `dvb-si`, `transmux` |
| `nrta_gyt_422_2025_tv_multi_audio_transmission.pdf`, `nrta_gyt_270_2013_dtv_closed_caption_system.pdf` | GY/T 422 audio preselection, GY/T 270 closed captions | `dvb-si`, `cc-data`, `caption-convert` |
| `nrta_gyt_282_2014_*`, `nrta_gyt_377_2023_*`, `nrta_gyt_379_2023_*`, `nrta_gyt_262_2012_*` | Chinese loudness standards | `broadcast-loudness`, `compliance-probe` |

Download URLs (browser User-Agent required; bare `curl` is blocked by ETSI):
- https://www.etsi.org/deliver/etsi_en/300400_300499/300468/01.19.01_60/en_300468v011901p.pdf
- https://www.etsi.org/deliver/etsi_ts/102000_102099/102006/01.07.01_60/ts_102006v010701p.pdf
- https://www.etsi.org/deliver/etsi_en/301100_301199/301192/01.07.01_60/en_301192v010701p.pdf
- https://www.etsi.org/deliver/etsi_ts/102300_102399/102323/01.04.01_60/ts_102323v010401p.pdf
- https://www.etsi.org/deliver/etsi_ts/102800_102899/102809/01.03.01_60/ts_102809v010301p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102772/01.01.01_60/ts_102772v010101p.pdf
- https://www.etsi.org/deliver/etsi_en/303500_303599/303560/01.01.01_60/en_303560v010101p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102727/01.01.01_60/ts_102727v010101p.pdf
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101202/01.02.01_60/tr_101202v010201p.pdf
- https://www.etsi.org/deliver/etsi_en/302300_302399/30230701/01.04.01_60/en_30230701v010401p.pdf
- https://www.etsi.org/deliver/etsi_en/302300_302399/30230702/01.04.01_60/en_30230702v010401p.pdf
- https://www.etsi.org/deliver/etsi_en/302700_302799/302755/01.04.01_60/en_302755v010401p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102773/01.04.01_60/ts_102773v010401p.pdf
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101290/01.04.01_60/tr_101290v010401p.pdf
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101211/01.09.01_60/tr_101211v010901p.pdf
- https://dutchguild.nl/event/13/attachments/82/203/SCTE_35_2023r1.pdf (public mirror; canonical source is the SCTE standards library request form)
- https://dvb.org/wp-content/uploads/2021/02/A001r18_Use-of-Video-and-Audio-Coding-in-Broadcast-and-Broadband-Applications_Draft_TS_101-154-v271_Nov-2021.pdf

## Consulted locally (gitignored, not redistributed)

- **Rec. ITU-T H.222.0 (06/2021)** = ISO/IEC 13818-1 (MPEG-2 Systems), free-of-charge ITU-T text. Downloaded from <https://www.itu.int/rec/T-REC-H.222.0>; kept as `specs/iso_iec_13818-1_2021_systems_itu_h222.0.pdf` (gitignored). Source of the MPEG-2 descriptor transcriptions (`dvb-si/docs/.../iso_13818_1/`) incl. Table 2-34 (stream_type) and the 0x02–0x3F descriptor syntax (#235–#237).
- **NorDig Unified Requirements ver. 3.1.1** — freely downloadable but copyright NorDig (no redistribution grant). From <https://nordig.org/> (v3.1.1 mirror <https://2rn.ie/wp-content/uploads/2022/03/NorDig-Unified-Requirements-ver.-3.1.1.pdf>); kept as `specs/nordig_unified_requirements_v3.1.1.pdf` (gitignored). Source of the NorDig private-descriptor namespace (PDS `0x00000029`): logical_channel_descriptor v1/v2 §12.2.9 etc. (#239).
- **CI Plus Specification v1.4.3 (2017-11)** — freely downloadable but its notice forbids reproduction (© CI Plus LLP). From <https://www.ci-plus.com/> / DigiCert KB; kept as `specs/ci_plus_specification_v1.4.3.pdf` (gitignored). CICAM comms-resource descriptors (cicam_virtual_channel, hybrid, IP_multicast) — limited SI relevance; consulted for the CI Plus namespace evaluation (#239). NOTE: the EN 300 468 ext-tag 0x14 "CI ancillary data descriptor" is defined in **EN 300 468 (Table 112)**, *not* here.

- **AVS3 / NRTA audio specs (2026-10-15)** — T/AI 109.7-2024, T/AI 109.3-2023 (aitisa.org.cn), GY/T 363-2023, GY/T 316-2018 (nrta.gov.cn): all free public downloads (no login), kept in the private submodule (`private/specs/`, text in `private/specs-md/`). Source URLs in `INCOMING-MANIFEST.md`.
