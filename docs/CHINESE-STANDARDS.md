# Chinese broadcast / AVS standards: acquisition index

Status: 2026-10-15. Scope: every **free, login-less, captcha-less** Chinese (and Chinese-originated) broadcast / audio-video standard that could serve a crate in this workspace, plus what is still missing and exactly what blocks it. Nothing in this document implements anything; section 6 lists follow-up opportunities.

All PDFs live in the **private submodule** (`private/specs/`), converted Markdown in `private/specs-md/` (index: `private/specs-md/INDEX-chinese-standards.md`), English working translations in `private/specs-md-en/` (index: `private/specs-md-en/INDEX.md`). The translations are unofficial derivatives of copyrighted text and therefore stay private. Per-file source URL, licence/free status and crates served: `specs/INCOMING-MANIFEST.md` (section "2026-10-15 Chinese broadcast / AVS standards acquisition").

## 1. The AVS3 TS carriage, definitively (what the DVB documents point at)

ETSI EN 300 468 v1.20.1 and TS 101 154 v2.10.1 delegate the AVS3 elementary-stream signalling to Chinese group standards. The cited documents and what they actually define:

| Item | Where defined | Value |
|---|---|---|
| AVS3 video `stream_type` | T/AI 109.6-2022 cl. 9.1; GY/T 420-2025 cl. 7.3.1; T/UWA 012.2-2023 cl. 5.3 | **0xD4** |
| AVS3 (Audio Vivid / AVS3-P3) audio `stream_type` | T/AI 109.7-2024 cl. 8.1.1; GY/T 420-2025 cl. 7.4.1; T/UWA 012.2-2023 cl. 5.3 | **0xD5** |
| AVS2 video / audio `stream_type` | GY/T 420-2025 cl. 7.2.1; T/AI 109.7-2024 cl. 8.2.1; T/UWA 012.2 | **0xD2** / **0xD3** |
| PES `stream_id` for AVS3 video and audio | T/AI 109.6 cl. 9.2.1; T/AI 109.7 cl. 8.1.2 | `1111 1101` (0xFD, extended_stream_id), `stream_id_extension_flag` = 0 |
| `stream_id_extension` | same | video main bitstream **0x41**, video library (knowledge) bitstream **0x42** (0x41-0x4F reserved range); audio **`100 1111` (0x4F)** |
| AVS2 video `stream_id` | GY/T 420 cl. 7.2.1; T/UWA 012.2 cl. 5.2 | `1110 xxxx` |
| `registration_descriptor` format_identifier | T/AI 109.6 / GY/T 420 cl. 7.3.3.1; T/AI 109.7 cl. 8.1.3.3 | video `0x41565356` ('AVSV'), audio `0x41565341` ('AVSA') |
| `AVS3_video_descriptor` tag | T/AI 109.6 Table 8; GY/T 420 Table 6 | **209 (0xD1)**, 8-byte body (descriptor_length 8) |
| `AVS3_audio_descriptor` tag | T/AI 109.7 Table 3; GY/T 420 Table 10 | **210 (0xD2)**, body depends on `audio_codec_id` (0 = general high-rate, 1 = lossless, 2 = general full-rate) |
| `AVS2_video_descriptor` tag | GY/T 420 cl. 7.2.3; T/UWA 012.2 cl. 5.5 | 64 (0x40) |
| `data_stream_alignment_descriptor` | T/AI 109.6 cl. 9.3.4 | H.222 tag 6; alignment_type 01 AU / 02 slice / 03 AU-or-slice / 04 sequence |
| Audio frame container | T/AI 109.3-2023 Annex A (= GY/T 363-2023 Annex A) | `aatf_frame()` (transport) / `aasf_header()`+`aasf_sequence()` (storage) |
| T-STD | T/AI 109.6 cl. 9.5; T/AI 109.7 cl. 8.1.4; GY/T 420 cl. 7.3.5 / 7.4.4 | TBn 512 B; Rx = BitRate x 400 (video); audio Rx = 1.2 x Rmax x N with Tables 7/8 |

How the DVB documents bind to this (TS 101 154 v2.10.1, clauses 4.1.5.x, 4.1.8.34, 4.1.8.35): they say the `AVS3_video_descriptor` "as defined in T/AI 109.6" and the `AVS3_audio_descriptor` "as defined in T/AI 109.7" shall be in the PMT ES loop, **preceded by a `private_data_specifier_descriptor`** with private_data_specifier 0x41565356 ('AVSV') / 0x41565341 ('AVSA'). The Chinese documents instead call for a `registration_descriptor` (tag 0x05) carrying the same four-character code. These are different mechanisms; a receiver that follows DVB looks for tag 0x5F, a Chinese-profile stream carries tag 0x05. A parser should accept both.

### Discrepancies found in the sources (read these before implementing)

1. **T/AI 109.6-2022 Table 9 omits `colour_primaries`.** The printed descriptor table (verified against the rendered PDF page 28) goes `reserved(2)`, `transfer_characteristics(8)`, `matrix_coefficients(8)`, `reserved(8)`, although clause 9.3.3 defines `colour_primaries`. GY/T 420-2025 Table 6 lists `colour_primaries` between the flag byte and `transfer_characteristics`, giving the 8-byte body above. Use the GY/T 420 layout.
2. **T/UWA 012.2-2023 (English) defines a different `AVS3_video_descriptor`**: tag value **62 (0x3E)**, only `reserved(3)` after `library_stream_flag` (no `library_picture_enable_flag`), and, when `library_stream_flag` is 0, a trailing `num_ref_library_stream(7)`, `id_type_flag(1)` plus a list of `ref_library_stream_PEID` (13) / `ref_library_stream_id` (8). That is an earlier, UWA-private layout; it conflicts with the later T/AI 109.6 / GY/T 420 tag 209 definition that DVB cites. T/UWA 012.2 also defines the extended PES `TimeStamp` structure (syncword 0xFEE, version 1, utc_time_valid, 48-bit utc_time in ms since 1970) carried in PES private data, and `stream_id` 1101 1101 for T/UWA 009.1 audio, which the later documents drop in favour of 0xFD/0x4F.
3. **`td_mode_flag`** is described as 2 bits in the text of T/AI 109.6 and GY/T 420 but is 1 bit in both tables (the tables add up to whole bytes only with 1 bit).
4. **Frame rate codes**: the descriptor tables (GY/T 420 Table 7, T/AI 109.6 Table 10) cover codes 0001-1010; the codec (T/AI 109.2 Table 48) also defines 1011 = 200, 1100 = 240, 1101 = 300, 1110 = 119.88 fps and reserves only 1111.
5. **GY/T 420 AVS2 sample entry** is called 'avsv', 'avst' and 'avs2' in different clauses; the box syntax uses 'avst'.
6. **`codecs` string**: `avs3.<profile_id hex>.<level_id hex>` (e.g. `avs3.22.54` = Main 10bit, level 8.0.60) and `av3a.<audio_codec_id>` (`av3a.02`); AVS2 uses `avst.<profile>.<level>` in GY/T 420 but `cavs.<audio_codec_id>` for AVS2 audio in T/AI 109.7.
7. **Audio descriptor branches**: T/AI 109.7 Table 4 has an `audio_codec_id == 0` branch (bitrate_index, bitstream_type, channel_number_index, raw_frame_length) that GY/T 420 Table 10 does not; GY/T 420 only covers codec ids 1 and 2.

## 2. Edition tracking

Rule applied (owner request): fetch a later edition only if it defines or changes the AVS3 TS carriage, the descriptors, the AVSA/AVSV specifier/registration, stream_type/stream_id/PES packaging, or the AATF/AASF header that TS 101 154 §6.10 constrains. Everything else newer was skipped and recorded here.

| Standard | Editions that exist (found) | Fetched | Skipped and why | DVB citation (what to match) |
|---|---|---|---|---|
| T/AI 109.2 (AVS3 video) | 2021 only; the site's file path is dated 2024-11-22 (re-uploaded under the same designation). Industry-standard restatement: GY/T 368-2023 | 109.2-2021 (the 2024-11 file), GY/T 368-2023 | no newer edition found | TS 101 154 v2.10.1 [55] cites **T/AI 109.2-2021** (dated) |
| T/AI 109.3 (immersive audio, AATF/AASF) | 2023 (the Chinese release file is named "发布版20230309"; English edition 2023). Industry-standard restatement: GY/T 363-2023 (single edition, NRTA list shows no amendment) | Chinese and English 109.3-2023; GY/T 363 Chinese (parallel agent) and its official English edition from UWA | AITISA's own English copy equals the vendored English text apart from the equation text layer; not duplicated | EN 300 468 v1.20.1 [58] and TS 101 154 v2.10.1 [58] cite **T/AI 109.3 undated** -> the 2023 edition is the only one |
| T/AI 109.6 (intelligent media format; AVS3 video TS/CMAF/DASH/SMT) | 2022 (implemented 2022-12-30) only. Restated in GY/T 420-2025 | 109.6-2022; GY/T 420-2025 | no newer edition | TS 101 154 v2.10.1 [56] cites **T/AI 109.6-2022** (dated). Note: this, not "109.6 = video", is where the AVS3_video_descriptor body lives; the video codec is 109.2 |
| T/AI 109.7 (audio encapsulation and transport; AVS3_audio_descriptor) | 2024 (implemented 2024-11-29) only | 109.7-2024 (byte-identical to the AITISA download) | no newer edition | EN 300 468 v1.20.1 [59], TS 101 154 v2.10.1 [59] cite **T/AI 109.7 undated** -> 2024 edition |
| T/AI 114 (intelligent media transport, SMT) | 2021 first edition; 2024 revision | 2024 revision (SMT underlies 109.6 cl. 8 and GY/T 420 cl. 8; borderline under the rule, kept because the SMT signalling of AVS3 cannot be read without it). The 2021 edition was downloaded for comparison and discarded | 2021 edition superseded | not cited by DVB |
| T/AI 109.4, 109.10, 121, 122, 125 | 109.4-2021 (conformance), 109.10-2024 (speech), 121/122-2023 (AVS3 encoder/decoder), 125-2023 (VR audio) | none | unrelated to carriage/descriptors | not cited |
| T/UWA 005.1 (HDR Vivid metadata) | 2022 v1.0 (2022-02-10); national-standard successor GB/T 46269.1-2025 exists (cited in GY/T 427-2026) | 005.1-2022 | GB/T 46269.1-2025 not obtainable (view-only) | EN 300 468 v1.20.1 names "T/UWA 005 DMI" (component types 0x0A); TS 101 154 v2.10.1 [61] **T/UWA 005.1** undated, [62] **T/UWA 005.2-1** undated |
| T/UWA 005.2-1 | 2026 edition (2026-09-21) found on UWA site | 2026 edition | none | [62] undated -> 2026 edition is the current one |
| GY/T 255 | 2012 and 2024 revision | both | none | not cited |
| GY/T 363, 368, 420 | single editions each; NRTA list shows amendment notices ("修改单") only for GY/T 399-2024 and 408-2024 | all | none | n/a |

Not a numbered edition but worth knowing: the AVS Workgroup page (avs.org.cn) still lists AVS3 part 2 as "awaiting national promulgation" in its English copy; no GB/T edition of AVS3 video or audio was found, only the NRTA industry standards GY/T 368-2023 / 363-2023 and the T/AI group standards.

## 3. What we have, ranked

Rank = value to this workspace now. "Tr." = English working translation exists in `private/specs-md-en/`; English originals need none.

**Rank 1: AVS3/Audio Vivid TS carriage and descriptor bodies (implement first)**
- T/AI 109.7-2024 (Tr.), T/AI 109.6-2022 (Tr.), GY/T 420-2025 (Tr.; also the ISOBMFF/FLV/DASH/SMT boxes), T/UWA 012.2-2023 (English), T/AI 109.3-2023 Chinese + English, GY/T 363-2023 Chinese + official English (AATF/AASF Annex A), T/UWA 009.1-2023 (English, 374 pp, Audio Vivid), T/AI 109.2-2021 (Tr. subset) and GY/T 368-2023 (AVS3 sequence header, profile/level tables).

**Rank 2: HDR Vivid / HDR metadata (DVB component type 0x0A "T/UWA 005 DMI")**
- T/UWA 005.1-2022, GY/T 358-2022 (English edition), T/UWA 005.2-1-2026, 005.2-2, 005.3-3/4/5 (English), GY/T 419-2025, UWA HDR Vivid white paper, GY/T 427-2026.

**Rank 3: signalling descriptors with a DVB counterpart**
- GY/T 422-2025 (Tr.; Chinese audio_preselection_descriptor), GY/T 270-2013 (Tr. subset; China closed-caption profile: stream_type 0x80, tag 0x86, T.35 0x26/0x0031), GY/T 301-2016 (subtitle data format).

**Rank 4: loudness (`broadcast-loudness`, `compliance-probe`)**
- GY/T 262-2012 (BS.1770-2 algorithm), 263-2012, 282-2014 (Tr.), 377-2023 (Tr.), 379-2023 (Tr.): targets -24 LKFS +/-2 LU and max -2 dBTP for DTV, -15 or -24 LKFS +/-2 LU and max -1 dBTP for online, monitoring rules (10 min / 3 s units, -22 LKFS exceedance ratios M10/S3).

**Rank 5: codecs and container mappings**
- GY/T 299.1-2016 (AVS2), GY/T 257.1-2012 (AVS+), GY/T 398.1-2024 (light-compression video), GY/T 365-2023 (4K programme file format, MXF mapping), GY/T 404-2024 (audio definition model), T/AI 114-2024 (SMT).

**Rank 6: IP production and audio-over-IP (ST 2110 / AES67 / ST 2022-7 equivalents)**
- GY/T 414/415/416/417-2024, GY/T 372-2023, GY/T 304-2016, GY/T 348-2021 (PTP profile), GY/T 322.1-3-2019 (OCA/AES70), GY/T 347.1-3-2021 (UHDTV SDI).

**Rank 7: conditional access, DRM, legacy and mobile broadcast (reference value, no crate yet)**
- GY/Z 175-2001 (DTV CAS, SimulCrypt-like), GY/T 255-2012/2024, 308-2017 (downloadable CAS), GY/T 246-2020 and 333-2020 (IPTV/cable DRM), GY/T 220.1-10 (CMMB), GY/T 268.1-2 (CDR), GY/T 214-2006, GY/T 226-2007, 170-2001, 338-2020, 286/287, 341-2020, 212-2017, 219-2006, 163/161-2000, 168-2001, 281-2014, 307-2017, 360-2022.

## 4. Blocked, missing, skipped (and exactly why)

| Item | What blocks it |
|---|---|
| GB/T 22726-2008 DRA audio (and the DRA TS signalling) | SAC `openstd.samr.gov.cn` offers online preview only ("在线预览"), no download link; T/AI 103-2016 (AVS audio) links to `c.gb688.cn` online view, same. Only the IANA `audio/vnd.dra` registration (sync word 0x7FFF, sample-rate table reference) and the IETF draft `draft-xu-avt-dra-00` (RTP payload) are freely readable; no free normative text found |
| GB/T 20600-2006 DTMB, DTMB-A | SAC view-only; paid copies only (ANSI, chinesestandard.net). No free DTMB-A text found anywhere |
| GB/T 28161-2011 (digital TV broadcasting service information, the Chinese SI/PSI spec; replaces GY/Z 174-2001) | SAC view-only. GY/Z 174 and the GY/T 229 series are **not in NRTA's list** of 387 published standards at all |
| GB/T 17975.1-2010 (MPEG-2 systems), GB/T 20090.x (AVS1), GB/T 33475.x (AVS2), GB/T 41808/41809-2022, GB/T 46269.1-2025 (HDR Vivid national standard) | SAC view-only |
| ttbz.org.cn (national group-standard platform: many T/AI, T/UWA documents) | the site serves a WAF slider captcha ("WEB 应用防火墙") to browsers and its `PdfFileStreamGet` links return a 404 JSON to scripted fetches; not bypassed. Every T/AI and T/UWA document found there was available from the issuing body instead |
| avs.org.cn standards pages | list standards and plan numbers only; downloads are member/FTP-gated (stated on the page) |
| UWA T/UWA 012.1-2023 (programme broadcasting) | the link on the UWA resources page returns an HTML page, not a PDF (appears form-gated); not pursued |
| GY/T 106-1999, GY/T 361-2022, GY/T 362-2022 | downloadable but image-only PDFs (no text layer); not vendored (would need OCR; low value) |
| GY/T 129-1997, GY/T 120-1995 | NRTA article pages carry no download link |
| Chinese SCTE-35 / SCTE-104 equivalents (cueing) | none found in the NRTA list; the only cueing-like items are GY/T 361-2022 (image-only) and GY/T 360-2022 (content tags) |
| CUVA / UWA hardware test specs (T/UWA 005.3-1/-2/-6, 012.5/.7/.8/.9, LED/display) | downloadable but irrelevant to any crate; not vendored |
| NRTA standards unrelated to AV data formats (cable plant, towers, tapes, ...) | out of scope (the full 387-entry list was enumerated; see manifest for what was taken) |

## 5. English working translations

Index with clauses covered, skipped, verification: `private/specs-md-en/INDEX.md`. Done: GY/T 420-2025, T/AI 109.6-2022, T/AI 109.7-2024, T/AI 109.2-2021 (subset), GY/T 422-2025, GY/T 270-2013 (clauses 6-9), GY/T 282-2014, 377-2023, 379-2023 (plus the parallel agent's GY/T 363 Annex A and GY/T 316). Method: clause-by-clause from the textlayer Markdown; every number/hex/binary literal checked against the Chinese Markdown; key syntax tables compared with the rendered PDF page; source defects are marked in the text, not silently fixed. **Not yet translated** (queue, in value order): GY/T 368-2023 beyond the sequence header, GY/T 365 (MXF mapping), GY/T 414-417 (IP production), GY/T 404 (ADM), GY/Z 175 + GY/T 255/308 (CAS), GY/T 301, GY/T 419, GY/T 220.2/.3, the remainder of GY/T 270 (clauses 10-11, CEA-708 style), the DVB-relevant parts of T/AI 114-2024.

## 6. Follow-up implementation opportunities (do NOT implement here)

- **dvb-si**: typed `AVS3_video_descriptor` (tag 0xD1, 8-byte body, layout per GY/T 420 Table 6) and `AVS3_audio_descriptor` (tag 0xD2; branches on `audio_codec_id` 0/1/2; GY/T 420 tables for 1/2, T/AI 109.7 Table 4 for 0), `AVS2_video_descriptor` (0x40), the `private_data_specifier` 'AVSV'/'AVSA' handling that TS 101 154 4.1.8.34/35 requires and the tag-0x05 registration variant the Chinese documents use, the UWA tag-62 layout as an alternative decode; GY/T 422 audio preselection (Chinese layout of the EN 300 468 ext 0x19 descriptor: compare field widths with the DVB one before reuse); China `caption_service_descriptor` (0x86, `char_set` table GB 2312 / GB 13000.1 / GB 18030); CA descriptors from GY/Z 175.
- **mpeg-ts / mpeg-pes**: `StreamType` 0xD2, 0xD3, 0xD4, 0xD5 and 0x80 (caption PES), stream_id 0xFD with `stream_id_extension` 0x41 / 0x42 / 0x4F, extended-stream-id PES header parsing, the UWA extended PES `TimeStamp` (0xFEE) private data.
- **transmux**: AVS2/AVS3 sample entries and configuration boxes (`avst`/`av2c`/`la2c`, `avs3`/`av3c`/`lav3`/`lavc`), `lrap`/`a3lg`/`telg` sample groups and the `lidx` box, restricted entry 'resv' with scheme 'av3l', `a3lr`/`ltrf` track references, Audio Vivid `av3a`/`a3as` entries with `dca3`, CMAF brands `ca3v`/`ca3a`, DASH descriptors (`urn:avs:avs3:p6:2022:*`, `urn:avs:avs3:p7:2024:audio_channel_configuration`), AVS3 sequence-header parse for `Avs3DecoderConfigurationRecord` (T/AI 109.2 sequence_header), HDR Vivid SEI/T.35 carriage (terminal_provide_code 0x0004 / oriented 0x0005 per TS 101 154 and T/UWA 005.2-1), SMT as a future output from T/AI 114-2024.
- **broadcast-hls / rtmp-runtime**: `CODECS` strings `avs3.<profile>.<level>` and `av3a.<id>`; FLV `CodecID` 13 (AVS2) / 14 (AVS3), audio `SoundFormat` 9 with `SoundFormatEx` 17 (Audio Vivid) and `AudioPacketType` 0 = `CA3SpecificBox`, 1 = `aatf_frame()`.
- **rtp-packet / SDP users**: RTP payload `AV3A-AATF` (T/AI 109.7 cl. 10.1: PT 96-127, default 90 kHz clock, marker = last fragment, `codec-nn-id`/`config`/`bitrate` fmtp), plus the Chinese ST 2110/AES67 equivalents GY/T 414-417, 304, 372.
- **broadcast-loudness / compliance-probe**: compliance profiles for GY/T 282 (-24 LKFS +/-2 LU, -2 dBTP) and GY/T 377 (-15/-24 LKFS +/-2 LU, -1 dBTP) and the GY/T 379 monitoring statistics (M10 / S3 over -22 LKFS).
- **cc-data / caption-convert**: China profile of caption carriage (GY/T 270: PES stream_id 0xBD with stream_type 0x80, `DVB1_data` 0x03 / "GA94" user data, T.35 country 0x26 provider 0x0031, constant 9600 bps cc_count table), GY/T 301 subtitle data format as a conversion source.
- **st377-1 / st291 / st12-1**: GY/T 365 MXF mapping, GY/T 163/219 timecode, GY/T 347 SDI payload IDs.
- **dvb-simulcrypt / dvb-csa / dvb-ci**: read GY/Z 175-2001 and GY/T 255/308 for the Chinese SimulCrypt-like interfaces before deciding scope.
- **New-crate candidates (docs now exist, nothing written)**: AVS3-P3 / Audio Vivid bitstream (AATF/AASF headers from GY/T 363 and T/AI 109.3), CMMB (GY/T 220.x), CDR (GY/T 268.x), SMT (T/AI 114).
