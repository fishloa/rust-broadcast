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
