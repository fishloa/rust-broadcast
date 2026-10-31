# Standards matrix

Every standard the workspace implements or relies on, which crate implements
it, which edition, how completely, and where the spec transcription lives. For
the per-crate view see [CRATES.md](CRATES.md).

**Transcription locations.** All spec material the rows below name (`<crate>/docs/...`, `specs/...`: transcriptions, notes, registries, RFC/IETF/W3C texts) now lives only in the private companion repo, under `private/specs-verified/<family>/<crate>/...`. Each crate's `docs/README.md` points there, and [`specs/PRIVATE-MAP.md`](specs/PRIVATE-MAP.md) maps every old public path to its private path.

**How this table was built.** Each row comes from the implementing crate's
module docs (the `//!` citation the code carries), its spec transcription
header, or the vendored spec lists (`specs/README.md`,
`CHINESE-STANDARDS.md`). Nothing is inferred from the
standard's name. Where the edition is not stated in the code or its
transcription, the row says **unverified** rather than guessing. When you
change a crate's coverage, update its row here in the same PR.

**Status vocabulary** (the same one crate READMEs use, see
[DOC-STANDARD.md](DOC-STANDARD.md#status-vocabulary)):

| Status | Meaning |
|---|---|
| **Full** | Every syntax structure in the cited scope parses and serializes, typed, with round-trip tests. |
| **Partial** | Some of the standard's structures are implemented; the Notes column says which. |
| **Typed-opaque** | The framing is typed, but a payload the standard defines is kept as bytes on purpose (for example an encrypted or codec payload). |
| **Label-only** | Only identifiers (tags, type codes, names) are known; there is no parser for the structure. |
| **Reference** | Used for constants, validation or semantics, not implemented as a wire format. |
| **Not implemented** | Transcribed or planned, but no code. |

**Where transcriptions live.** "Transcription" paths are the old public
paths; every one of them is private-only now and is decoded by `docs/specs/PRIVATE-MAP.md`.

## ETSI / DVB

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| ETSI EN 300 468 | DVB Service Information | V1.20.1 (baseline V1.19.1 plus the delta) | `dvb-si` | Full | `dvb-si/docs/` (tables, descriptors, enums, text), `dvb-si/docs/en300468-v1.20.1-delta.md` | Every allocated `table_id` (Table 2) and `descriptor_tag` (Table 12); Annex A text decoding. |
| ETSI TS 102 006 | System Software Update (SSU) | V1.4.1 (2015-06; the vendored file is named `v01.07.01` but its title page says V1.4.1) | `dvb-si` | Full (UNT + SSU selectors) | `dvb-si/docs/` | UNT (table_id 0x4B), `data_broadcast_id` 0x000A selector, GroupInfoIndication. |
| ETSI EN 301 192 | DVB data broadcasting | V1.7.1 | `dvb-si` | Partial | `dvb-si/docs/` | INT (0x4C) and MPE sections. |
| ETSI TR 101 202 | Data broadcasting implementation guidelines | V1.2.1 | `dvb-si` | Reference | `dvb-si/docs/text/` | DVB profile semantics for the carousel; byte syntax is ISO/IEC 13818-6. |
| ETSI TS 102 809 | Signalling of interactive applications (HbbTV AIT) | V1.3.1 | `dvb-si` | Partial | `dvb-si/docs/` | AIT (0x74) and protection message (0x7B). |
| ETSI TS 102 323 | TV-Anytime carriage | V1.4.1 | `dvb-si` | Partial | `dvb-si/docs/` | RCT (0x76), CIT (0x77), RNT (0x79), container (0x75). |
| ETSI TS 102 772 | MPE inter-burst FEC | V1.1.1 | `dvb-si` | Partial | `dvb-si/docs/` | MPE-IFEC section (0x7A). |
| ETSI EN 303 560 | TTML subtitling systems | V1.1.1 | `dvb-si` | Partial | `dvb-si/docs/` | Downloadable font info table (0x7C) only. |
| ETSI TS 102 727 | MHP 1.2.2 | V1.1.1 | `dvb-si` | Partial | `dvb-si/docs/` | `XAIT_location` descriptor only. |
| ETSI TR 101 211 | SI implementation guidelines | V1.9.1 | `dvb-si` | Reference | `dvb-si/docs/text/tr_101_211/` | Running-status semantics, repetition rates. |
| ETSI TS 101 154 | AV coding in broadcast/broadband | V2.6.1 (`cc-data` Table B.9); other crates: edition not recorded per crate (vendored: V2.6.1 (2019-09), DVB A001 Rev.18 draft V2.7.1 (Nov 2021), V2.10.1 (2026-05)) | `cc-data`, `dvb-si`, `transmux`, `mpeg-ts`, `mpeg-pes`, `dvb-conformance`, `caption-convert` | Partial | `cc-data/docs/ts_101_154/` | `cc_data()` carriage; component/stream-type semantics elsewhere. |
| ETSI TS 102 773 | DVB-T2 Modulator Interface (T2-MI) | V1.4.1 | `dvb-t2mi` | Full | `dvb-t2mi/docs/` | All packet types, individual addressing functions. |
| ETSI EN 302 755 | DVB-T2 | V1.4.1 | `dvb-bbframe`, `dvb-t2mi` | Partial | `dvb-bbframe/docs/`, `dvb-t2mi/docs/` | BBHEADER, ISSY (Annex C), L1 signalling fields. Physical layer (modulation, FEC) is out of scope. |
| ETSI EN 302 307-1 / -2 | DVB-S2 / DVB-S2X | V1.4.1 (2014) / -2 V1.4.1 (2024-08), both vendored | `dvb-bbframe` | Partial | `dvb-bbframe/docs/` | BBHEADER, MODCOD tables, CRC-8. Physical layer out of scope. |
| ETSI TR 101 290 | Measurement guidelines for DVB systems | V1.4.1 (2020-06) | `dvb-conformance`, `compliance-probe`, `media-doctor` | Partial | `dvb-conformance/docs/tr_101_290.md` | Priority 1 (1.1-1.6), priority 2 (2.1-2.3b, 2.5-2.6), priority 3 (3.1-3.10). Indicator 2.4 (PCR accuracy) not emitted. |
| ETSI EN 300 743 | DVB subtitling | V1.6.1 | `dvb-subtitle` | Full (segments) | `dvb-subtitle/docs/tables/en_300_743/` | Rendering is out of scope. |
| ETSI EN 301 775 | VBI data carriage in DVB | V1.2.1 §4 | `dvb-vbi` | Full (§4 PES data field) | `dvb-vbi/docs/vbi.md` | |
| ETSI EN 300 472 | Teletext (ITU-R System B) in DVB | V1.4.1 | `dvb-vbi`, `dvb-si` | Partial | — | Carriage only. |
| ETSI EN 300 706 | Enhanced Teletext | V1.2.1 | `timed-metadata` (`teletext`), `caption-convert` | Partial | `timed-metadata/docs/teletext-subtitles.md` | Subtitle-page decode rules only. |
| ETSI EN 300 231 / EN 300 294 | VPS / WSS | edition unverified | `dvb-vbi` | Partial | `dvb-vbi/docs/vbi.md` | Data-unit framing. |
| ETSI EN 50221 | Common Interface for conditional access | 1997 | `dvb-ci`, `dvb-ci-runtime` | Full (APDU/SPDU/TPDU + resources listed in the crate) | `dvb-ci/docs/en_50221/`, `dvb-ci-runtime/docs/` | |
| ETSI TS 101 699 | Extensions to the Common Interface | V1.1.1 | `dvb-ci`, `dvb-ci-runtime` | Partial | `dvb-ci/docs/ci_plus/` | |
| ETSI TS 103 205 | Extensions to CI Plus | V1.4.1 | `dvb-ci` | Partial | `dvb-ci/docs/ts_103_205/` | Multi-stream CA support. The CI Plus core specification is not transcribed. |
| ETSI TS 100 289 | Scrambling control semantics | V1.1.1 (2011-09), vendored | `mpeg-ts`, `dvb-csa` | Reference | `mpeg-ts/docs/tables/scrambling_control.md` | |
| DVB-CSA (CSA2) | Common Scrambling Algorithm | n/a (no public normative text) | `dvb-csa` | Full (cipher) | — | Validated against libdvbcsa known-answer vectors. |
| ETSI TS 103 197 | DVB SimulCrypt head-end | V1.5.1 | `dvb-simulcrypt` | Partial | `dvb-simulcrypt/docs/` | Generic message, ECMG<->SCS, EMMG/PDG<->MUX, C(P)SIG<->(P)SIG. Other interfaces not implemented; CW/ECM/EMM payloads opaque. |
| ETSI TS 103 769 | DVB multicast ABR | V1.2.1 clause 10 | `dvb-mabr` | Partial | `dvb-mabr/docs/` | Session-configuration XML only; architecture, reporting and transport clauses transcribed but not implemented. |
| ETSI TS 103 752-1 | DVB targeted advertising (binary SCTE 35 profile) | V1.2.1 | `scte35-splice` | Partial | `scte35-splice/docs/dvb_ta/` | |
| ETSI TS 103 285 | MPEG-DASH profile for DVB | V1.4.1 | `transmux`, `multimux` | Reference | — | |
| ETSI TS 102 366 | AC-3 / E-AC-3 | V1.4.1 | `transmux`, `dvb-si` | Partial | `transmux/docs/codec/` | Sync frame, `bsi()`, ISOBMFF Annex F boxes. No audio decoding. |
| ETSI TS 103 190-1 / -2 | AC-4 | V1.4.1 / V1.3.1 | `dvb-si`, `transmux` | Partial | `transmux/docs/codec/ac4-dac4.md` | Descriptor and `dac4` box; `ac4_toc` not typed. |
| ETSI TS 102 114 | DTS Coherent Acoustics | V1.6.1 | `transmux` | Partial | `transmux/docs/codec/dts-core-frame.md` | Core frame header and ISOBMFF `ddts`. |
| ETSI TS 102 034 | MPEG-2 TS services over IP | V2.1.1 | — | Reference | — | Vendored for `dvb-stream`/`multimux` context; not cited in code. |
| NorDig Unified Requirements | Logical channel descriptors | v3.2.2 (2025-11), vendored (`nordig_unified_requirements_ird_v3_2_2_2025-11`; §12.2.9, Table 12.12, tags 0x83 and 0x87) | `dvb-si`, `dvb-tools` | Partial | — | LCD v1 and v2 private descriptors. |
| EACEM / D-Book | Logical channel descriptor (tag 0x83) | not vendored; edition unverified | `dvb-si` | Partial | — | The shared EACEM/NorDig/D-Book layout of tag 0x83 only. |

## ISO/IEC and ITU-T

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| ISO/IEC 13818-1 = ITU-T H.222.0 | MPEG-2 Systems | H.222.0 (08/2023) for `mpeg-ts`; ISO/IEC 13818-1:2021 (06/2021) for `mpeg-pes`/`mpeg-ps` | `mpeg-ts`, `mpeg-pes`, `mpeg-ps`, `dvb-si`, `dvb-conformance`, `ts-fix`, `transmux`, `media-doctor` | Full for TS packet, adaptation field, PSI sections, PES, PS; Partial T-STD model | `mpeg-ts/docs/`, `mpeg-pes/docs/tables/`, `mpeg-ps/docs/tables/`, `dvb-conformance/docs/h222_0-tstd-buffer-model.md` | MPEG-1 PS packs are rejected, not parsed. |
| ISO/IEC 13818-6 | DSM-CC | edition unverified | `dvb-si`, `dvb-ci`, `transmux` | Partial | `dvb-si/docs/text/iso_13818_6/` | Data/object carousel (DSI/DII/DDB, BIOP) and module reassembly. |
| ISO/IEC 14496-12 | ISO Base Media File Format | edition unverified (the `prft` transcription quotes 14496-12:2015) | `transmux`, `container-probe`, `media-doctor`, `hls-runtime`, `broadcast-hls` | Partial | `transmux/docs/codec/isobmff-*.md` | The boxes needed for fMP4/CMAF/progressive MP4 mux and demux. Paid text, not vendored. |
| ISO/IEC 14496-15 | NAL-unit codecs in ISOBMFF | 2022 for `vvcC`; others unverified | `transmux` | Partial | `transmux/docs/codec/avcC-hvcC-14496-15.md`, `vvcC-14496-15.md` | `avcC`, `hvcC`, `vvcC` records. |
| ISO/IEC 14496-30 | Timed text in ISOBMFF | edition unverified | `transmux` | Partial | `transmux/docs/codec/captions-14496-30.md` | `stpp`/`wvtt`, structure matched to real fixtures. |
| ISO/IEC 14496-1 / -3 | MPEG-4 systems ES descriptor / AAC | edition unverified | `transmux` | Partial | `transmux/docs/codec/es-descriptor-14496-1.md`, `heaac-asc.md` | `esds`, AudioSpecificConfig incl. SBR/PS. |
| ISO/IEC 13818-7 | MPEG-2 AAC (ADTS) | 2004 | `transmux`, `container-probe` | Partial | `transmux/docs/codec/adts-error-check.md` | ADTS framing. |
| ISO/IEC 23000-19 | CMAF | edition unverified | `transmux` | Partial | — | CMAF mux and conformance validator. |
| ISO/IEC 23009-1 | MPEG-DASH (MPD, `emsg`) | edition unverified | `transmux`, `mp4-emsg`, `timed-metadata`, `media-doctor`, `multimux` | Partial | `mp4-emsg/docs/emsg.md` | MPD write (and validation in `media-doctor`); `emsg` v0/v1 Full. |
| ISO/IEC 23001-7 | Common Encryption (CENC) | edition unverified | `transmux`, `broadcast-common`, `broadcast-hls` | Partial | `transmux/docs/codec/cenc-23001-7.md` | `cenc` and `cbcs` encrypt/decrypt; scheme identity in `broadcast-common`. Paid text. |
| ISO/IEC 23008-3 | MPEG-H 3D Audio | edition unverified | `transmux` | Partial | `transmux/docs/codec/mpegh-mhaC.md` | `mhaC` record and TS carriage; no audio decoding. |
| ITU-T H.264 / H.265 / H.266 / H.262 | AVC / HEVC / VVC / MPEG-2 video | H.264 (08/2024), H.265 (01/2026), H.266 (01/2026), H.262 (1995) | `transmux` | Partial | `transmux/docs/codec/` | Parameter-set and SEI fields needed for container signalling only. Samples are never decoded. |
| ITU-R BS.1770 | Loudness measurement | BS.1770-5 (11/2023) | `broadcast-loudness` | Full | `broadcast-loudness/docs/bs1770-algorithm.md` | K-weighting at any sample rate, gating, true peak. |

## SCTE, CTA and ATSC

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| ANSI/SCTE 35 | Digital Program Insertion cueing | 2023r1 | `scte35-splice` (and consumers `timed-metadata`, `ssai-runtime`, `playout-runtime`, `media-doctor`, `compliance-probe`) | Full | `scte35-splice/docs/` | All splice commands and descriptors. |
| ANSI/SCTE 104 | Automation to compression API | 2023 | `scte104` | Full (messages + operations) | `scte104/docs/ansi_scte_104/` | |
| ANSI/SCTE 214-3 | SCTE 35 in DASH (`emsg`) | edition unverified | `timed-metadata`, `mp4-emsg`, `transmux` | Partial | — | §8.3.3 `emsg` carriage, scheme `urn:scte:scte35:2013:bin`. |
| ANSI/CTA-608-E | Line 21 data services | S-2019 | `cc-data` (`decode`), `caption-convert`, `timed-metadata` | Partial | `cc-data/docs/decode/cea608-decode.md` | Byte-pair decode model. |
| ANSI/CTA-708-E | DTV closed captioning | S-2023 | `cc-data` (`decode`), `caption-convert`, `timed-metadata` | Partial | `cc-data/docs/decode/cea708-decode.md` | Service-block decode model. |
| ATSC A/53 Part 4 | MPEG-2 video system characteristics (caption carriage) | 2009 | `transmux`, `cc-data` | Partial | `transmux/docs/codec/a53-caption-sei.md` | `GA94` user data / SEI extraction. |
| ATSC A/342-3 | MPEG-H system | A/342-3:2025-10 (17 October 2025), vendored | `transmux` | Reference | `transmux/docs/codec/mpegh-atsc-a342-3.md` | |
| ATSC A/331 | Signaling, delivery, synchronization and error protection | A/331:2026-04 (14 April 2026) vendored; `atsc3-route` code cites A/331:2025-06 | `atsc3`, `atsc3-route` | Partial (archived) | `atsc3/docs/` | LLS envelope and SLT; ROUTE header-extension constants, FEC payload IDs, codepoints. ATSC 3.0 work is abandoned. |
| ATSC A/321 | System discovery and signaling (bootstrap) | 2026-06 | — | Not implemented | `atsc3/docs/a321-bootstrap.md` | Transcription only. |

## SMPTE, AES, EBU, VSF and Dolby

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| SMPTE ST 291-1 | Ancillary data packet and space formatting | 2011 | `st291` | Full (packet) | `st291/docs/anc_packet_291.md` | |
| SMPTE ST 2038 | ANC data in MPEG-2 TS | 2021 | `st291` (`ts`) | Full | `st291/docs/st_2038.md` | |
| SMPTE ST 2110-40 / RFC 8331 | ANC data over RTP | ST 2110-40:2023 / RFC 8331 (2018) | `st291` (`rtp`) | Full | `st291/docs/anc_rtp_8331.md` | |
| SMPTE ST 2110-10 / -20 / -21 / -30 | Professional media over IP | 2022 | — | Not implemented | `st2110/docs/` | Docs-only directory. |
| SMPTE ST 2110-31 | AES3 transparent transport | 2022 | — | Reference | — | Context for `st337` over RTP. |
| SMPTE ST 12-1 | Time and control code (LTC) | 2014 §9 | `st12-1` | Full (codeword) | `st12-1/docs/st12-1.md` | ST 12-2 / 12-3 vendored for context only. |
| SMPTE ST 337 | Non-PCM audio and data in AES3 | 2015 | `st337` | Full (burst framing; payload opaque) | `st337/docs/st337.md` | |
| SMPTE ST 377-1 | MXF file format | 2019 | `st377-1`, `container-probe` | Partial | `st377-1/docs/st377-1.md` | Backbone (KLV, partitions, primer, structural metadata, RIP). Essence mappings and DM plug-ins out of scope. |
| SMPTE ST 336 | KLV data encoding | 2017 | `st377-1`, `transmux` | Partial | — | KLV framing. |
| SMPTE ST 378 | MXF Operational Pattern 1a | 378M-2004 | `st377-1` | Partial | `st377-1/docs/st378-op1a.md` | |
| SMPTE ST 2022-6 | HBRMT (SDI over IP) | 2012 | `st2022` | Full (payload header) | `st2022/docs/st2022-6-framing.md` | |
| SMPTE ST 2022-7 | Seamless protection switching | ST 2022-7:2019 (revision of ST 2022-7:2013), vendored | — | Not implemented | `st2022/docs/st2022-7-hitless.md` | |
| SMPTE RDD 29 | Dolby Atmos bitstream | 2019 | `rdd29` | Partial | `rdd29/docs/rdd29.md` | Frame/element framing, bed and object definitions, audio data DLC. |
| EBU R 128 | Loudness normalisation | November 2023 | `broadcast-loudness` | Full | `broadcast-loudness/docs/r128-targets.md` | |
| EBU Tech 3341 | Loudness metering (EBU Mode) | November 2023 | `broadcast-loudness` | Full | `broadcast-loudness/docs/r128-metering.md` | Compliance signals are the test gate. |
| EBU Tech 3342 | Loudness range | November 2023 | `broadcast-loudness` | Full | `broadcast-loudness/docs/tech3342-lra.md` | |
| VSF TR-06-1 | RIST Simple Profile | 2020 | `rist-runtime` | Partial | `rist-runtime/docs/tr-06-1-simple-profile.md` | RTCP messages and ARQ engine; no socket layer. |
| VSF TR-06-2 | RIST Main Profile | 2024 | — | Not implemented | `rist-runtime/docs/tr-06-2-main-profile-timing.md` | Transcription only. |
| MISB ST 0601 | UAS datalink local set | edition unverified | `transmux` | Partial | `transmux/docs/klv/klv-misb0601.md` | |

## IETF

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| RFC 3550 | RTP / RTCP | RFC | `rtp-packet` (§5), `rtcp-packet` (§6), `transmux`, `dvb-stream`, `webrtc-runtime`, `multimux` | Full | `rtp-packet/docs/rtp-header.md`, `rtcp-packet/docs/rtcp.md` | |
| RFC 8285 | RTP header-extension multiplexing | RFC | `rtp-packet` (`rfc8285`) | Full | `rtp-packet/docs/rfc8285_header_ext.md` | |
| RFC 4585 | RTCP feedback (Generic NACK) | RFC, §6.2.1 | `rist-runtime` | Partial | — | Generic NACK only. |
| RFC 6184 / RFC 3640 | RTP payload for H.264 / MPEG-4 AAC | RFC | `transmux`, `multimux` | Partial | `transmux/docs/rtp/rtp-payload-formats.md` | |
| RFC 4566 / RFC 8866 | SDP | RFC | `transmux`, `webrtc-runtime`, `multimux` | Reference | — | Built and parsed through the `sdp-types` crate, not hand-rolled. |
| RFC 2326 | RTSP 1.0 | RFC | `rtsp-runtime` | Partial | `rtsp-runtime/docs/` | Client and server state machines (Appendix A), Transport/Session headers, interleaving, auth. Message codec via `rtsp-types`. RTSP 2.0 (RFC 7826) not implemented. |
| RFC 7235 / RFC 7616 / RFC 2617 / RFC 6750 | HTTP auth: framework, Digest, Basic/Digest (legacy), Bearer | RFC | `broadcast-auth` | Partial | `rtsp-runtime/docs/auth.md` | Basic and Digest via `http-auth`; Bearer; a reverse-proxy `forwarded` scheme. |
| RFC 8216 / draft-pantos-hls-rfc8216bis | HTTP Live Streaming | RFC 8216 and draft -22 | `broadcast-hls`, `hls-runtime`, `transmux`, `timed-metadata`, `ssai-runtime`, `media-doctor` | Partial | `broadcast-hls/docs/` | Playlist syntax incl. LL-HLS, I-frames, interstitials (Appendix D). Known gaps are listed in the `broadcast-hls` crate docs. |
| RFC 9725 | WHIP | RFC | `webrtc-runtime`, `multimux` | Full (signalling) | `webrtc-runtime/docs/whip-rfc9725.md` | |
| draft-ietf-wish-whep | WHEP | draft -04 | `webrtc-runtime`, `multimux` | Full (signalling, per the draft) | `webrtc-runtime/docs/whep-draft-04.md` | |
| RFC 8445 / 8489 / 5764 / 3711 / 5761 | ICE, STUN, DTLS-SRTP, SRTP, RTCP mux | RFC | `webrtc-runtime` (`media`) | Reference | `webrtc-runtime/docs/` | Implemented by the external `rtc-*` crates; transcribed for review. |
| draft-sharabayko-srt | Secure Reliable Transport | draft -01 | `srt-runtime` | Partial | `specs/ietf_draft_sharabayko_srt_01.txt`, `specs/rules/srt-*.md` | Packets, HSv5 handshake, ARQ, TSBPD, LiveCC/FileCC, encryption. |
| RFC 4326 / RFC 5163 | ULE and its extension headers | RFC | `ule` | Full | `ule/docs/` | |
| RFC 5651 / 5775 / 6726 / 5740 | LCT / ALC / FLUTE / NORM | RFC | `rmt-flute`, `atsc3-route` | Partial | `rmt-flute/docs/` | Header and packet formats; FDT XML is not parsed. |
| RFC 5052 / RFC 6330 | FEC building block / RaptorQ | RFC | `rmt-flute`, `atsc3-route` | Partial | `rmt-flute/docs/fec.md` | FEC payload IDs and OTI only; no FEC encoding or decoding. |
| RFC 8794 / RFC 9559 | EBML / Matroska | RFC | `transmux`, `container-probe` | Partial | `transmux/docs/webm/ebml-matroska.md` | WebM demux/mux. |

## W3C, Adobe, Microsoft, DASH-IF and Apple

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| W3C TTML2 | Timed Text Markup Language 2 | REC 2018-11-08 | `ttml-subtitle` | Partial | `ttml-subtitle/docs/ttml2-syntax.md` | Element tree, style properties, time expressions. |
| W3C IMSC 1.1 | TTML profiles for subtitles and captions | REC 2018-11-08 (edited 2020-04-27) | `ttml-subtitle`, `caption-convert` | Full (profile validation) | `ttml-subtitle/docs/imsc11-profiles.md` | |
| W3C WebVTT | Web Video Text Tracks | edition unverified | `caption-convert`, `transmux` | Partial | — | Writer and WebVTT<->SRT. |
| Adobe RTMP 1.0 + AMF0 | Real-Time Messaging Protocol | Dec 2012 | `rtmp-runtime`, `transmux` | Partial | `rtmp-runtime/docs/rtmp.md`, `transmux/docs/rtmp/rtmp.md` | Publish/ingest only. |
| Adobe FLV | Flash Video file format | v10.1 Annex E | `transmux`, `container-probe` | Partial | `transmux/docs/codec/flv.md` | |
| Microsoft MS-SSTR | Smooth Streaming protocol | revision 11.0 (19 Nov 2024) per the vendored copy's revision summary | `transmux`, `multimux` | Partial | `transmux/docs/smooth/ms-sstr.md` | Manifest and fragments; PlayReady header via `transmux/docs/drm/pssh.md`. |
| DASH-IF IOP Part 10 | Events and timed metadata | V5.0.0 (2023-01) | `mp4-emsg` | Reference | `mp4-emsg/docs/emsg.md` | |
| DASH-IF CPIX | Content protection information exchange | 2.3 | — | Not implemented | `cpix/docs/` | Docs-only directory. |
| Apple HLS SAMPLE-AES | MPEG-2 stream encryption for HLS | version unverified | `transmux`, `broadcast-hls` | Partial | `transmux/docs/drm/hls-sample-aes.md` | |
| AOMedia AV1-ISOBMFF, WebM VP9-ISOBMFF, Opus/FLAC-in-ISOBMFF | Codec bindings in ISOBMFF | as published (free) | `transmux` | Partial | `transmux/docs/codec/` | Sample entries and config records only. |

## Chinese broadcast and AVS standards

Acquisition, edition tracking and the discrepancies between sources are in
CHINESE-STANDARDS.md. The PDFs and their Markdown are
private.

| Standard | Title | Edition implemented | Crate(s) | Status | Transcription | Notes |
|---|---|---|---|---|---|---|
| T/AI 109.6 | AVS3 intelligent media format (video system layer) | 2022 | `dvb-si`, `mpeg-pes`, `transmux` | Partial | private | `AVS3_video_descriptor` (tag 0xD1), `stream_type` 0xD4, PES `stream_id` 0xFD / extension 0x41-0x42. |
| T/AI 109.7 | AVS3 audio encapsulation and transport | 2024 | `dvb-si`, `mpeg-pes`, `rtp-packet` (`av3a`), `transmux` | Partial | private | `AVS3_audio_descriptor` (0xD2), `AVS2_audio_descriptor` (0xD3), `stream_type` 0xD5, AV3A-AATF RTP payload (§10.1). |
| T/AI 109.3 | AVS3 immersive audio (Audio Vivid) | 2023 | `rtp-packet` (`av3a`), `transmux` | Partial | private | AATF frame header (Annex A) only; no audio decoding. |
| T/AI 109.2 / GY/T 368 | AVS3 video coding | 2021 / 2023 | `transmux`, `dvb-si` | Partial | private | Sequence-header fields used for signalling. |
| GY/T 420 | UHD A/V stream encapsulation and network transmission | 2025 | `dvb-si`, `transmux` | Partial | private | AVS2/AVS3 video descriptors; AVS2, AVS3 and Audio Vivid in ISOBMFF (Annex A.2-A.4). |
| GY/T 299.1 | AVS2 video | 2016 | `dvb-si` | Reference | private | Field semantics for `AVS2_video_descriptor`. |
| T/UWA 012.2 | UHD system layer (TS encapsulation) | 2023 | `dvb-si` | Partial | private | The older AVS3 video descriptor (tag 0x3E). |
| T/UWA 005.1 / 005.2-1 (HDR Vivid) | HDR Vivid metadata | 2022 / 2026 | `dvb-si` | Label-only | private | Component-type labels only; the metadata itself is not parsed. |
| GY/T 270 | DTV closed caption system | 2013 | `dvb-si` | Partial | private | China `caption_service_descriptor` (tag 0x86). |
| GY/T 422 | TV multi-audio transmission | 2025 | `dvb-si` | Partial | private | The Chinese subset of the audio preselection descriptor. |
| GY/T 282 / GY/T 377 / GY/T 379 | Loudness requirements | 2014 / 2023 / 2023 | `broadcast-loudness` | Partial | private | Target profiles and monitoring helpers on top of BS.1770. |
