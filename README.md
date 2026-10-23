# rust-broadcast

[![CI](https://github.com/fishloa/rust-broadcast/actions/workflows/ci.yml/badge.svg)](https://github.com/fishloa/rust-broadcast/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#licence)

**Spec-grounded broadcast and streaming media protocols in Rust.** A workspace
of parsers, builders and sans-IO protocol engines covering DVB, MPEG-2 systems,
SCTE ad signalling, captions and subtitles, conditional access, container
remuxing (fMP4/CMAF/TS/HLS/DASH), live transport (RTSP, RTP, SRT, RTMP,
WebRTC) and SMPTE professional formats, plus a live origin server and
diagnostics tools built from them.

Every wire layout is cited to the clause of the standard it implements, has a
symmetric serializer, and is round-trip tested against real captures.

- **[docs/CRATES.md](docs/CRATES.md)** — every crate: what it does, its
  standards, `no_std` status, dependencies, and "which crate do I use for X".
- **[docs/STANDARDS.md](docs/STANDARDS.md)** — every standard: which crate
  implements it, which edition, how completely, and where the spec
  transcription lives.

## What can I do with it?

| Task | Crates | Try |
|---|---|---|
| **Read DVB service information** from a transport stream: services, EPG, networks, PMTs, carousels, software-update (SSU) chains | [`dvb-si`](dvb-si/) on [`mpeg-ts`](mpeg-ts/) | `cargo run -p dvb-tools -- services <file.ts>` |
| **Decode a DVB-T2 or DVB-S2 feed** down to service names: T2-MI, BBFrames, the inner TS | [`dvb-t2mi`](dvb-t2mi/), [`dvb-bbframe`](dvb-bbframe/), [`dvb-si`](dvb-si/) | `cargo run -p dvb-tools -- t2mi <file.ts> --inner` |
| **Repackage media**: TS, fMP4/CMAF, MP4, Program Stream, WebM, FLV in; CMAF, MP4, TS, HLS, LL-HLS, DASH, LL-DASH, Smooth out; CENC/CBCS encryption | [`transmux`](transmux/), [`broadcast-hls`](broadcast-hls/), [`container-probe`](container-probe/) | the `transmux` binary (feature `cli`) |
| **Run a live origin**: ingest RTSP, SRT, RTMP, TS over UDP/HTTP, HLS/DASH pull or a file; serve LL-HLS, DASH, Smooth, TS-HLS or catch-up; push to SRT/RTMP/RTSP | [`multimux`](multimux/), [`multimux-cli`](multimux-cli/), [`media-plane`](media-plane/) | `multimux --rtsp rtsp://cam.local/stream --name cam1` |
| **Build your own protocol endpoint** with sans-IO engines you drive from any runtime | [`rtsp-runtime`](rtsp-runtime/), [`srt-runtime`](srt-runtime/), [`rtmp-runtime`](rtmp-runtime/), [`hls-runtime`](hls-runtime/), [`webrtc-runtime`](webrtc-runtime/), [`rist-runtime`](rist-runtime/) | |
| **Check stream conformance and health**: ETSI TR 101 290, structural linting of TS/HLS/DASH/CMAF, live Prometheus metrics | [`dvb-conformance`](dvb-conformance/), [`media-doctor`](media-doctor/), [`compliance-probe`](compliance-probe/) | |
| **Repair a broken transport stream**: continuity counters, PCR restamp, PAT/PMT regeneration, PID filtering | [`ts-fix`](ts-fix/) | the `ts-fix` binary (feature `cli`) |
| **Insert ads with SCTE 35**: parse and build cues, translate them to HLS `EXT-X-DATERANGE` and DASH `emsg`, run server-side ad insertion or a scheduled linear channel | [`scte35-splice`](scte35-splice/), [`scte104`](scte104/), [`timed-metadata`](timed-metadata/), [`mp4-emsg`](mp4-emsg/), [`ssai-runtime`](ssai-runtime/), [`playout-runtime`](playout-runtime/) | |
| **Extract and convert captions**: CEA-608/708, teletext, DVB bitmap subtitles, TTML/IMSC, to WebVTT or SRT | [`cc-data`](cc-data/), [`dvb-vbi`](dvb-vbi/), [`dvb-subtitle`](dvb-subtitle/), [`ttml-subtitle`](ttml-subtitle/), [`caption-convert`](caption-convert/) | |
| **Talk to a CAM and descramble**: the EN 50221 Common Interface, DVB-CSA, SimulCrypt head-end messages | [`dvb-ci`](dvb-ci/), [`dvb-ci-runtime`](dvb-ci-runtime/), [`dvb-csa`](dvb-csa/), [`dvb-simulcrypt`](dvb-simulcrypt/) | |
| **Measure loudness** to EBU R 128 / ITU-R BS.1770-5 | [`broadcast-loudness`](broadcast-loudness/) | |
| **Handle studio formats**: MXF, SDI ancillary data, LTC timecode, AES3 non-PCM bursts, Dolby Atmos, SDI over IP | [`st377-1`](st377-1/), [`st291`](st291/), [`st12-1`](st12-1/), [`st337`](st337/), [`rdd29`](rdd29/), [`st2022`](st2022/) | |
| **Carry IP and files over broadcast**: ULE, FLUTE/ALC/NORM, DVB multicast ABR | [`ule`](ule/), [`rmt-flute`](rmt-flute/), [`dvb-mabr`](dvb-mabr/) | |

## The crates

Every workspace member, grouped by domain. The table below is generated from the
crates' manifests by [`tools/gen-crate-index.py`](tools/gen-crate-index.py);
[docs/CRATES.md](docs/CRATES.md) has the full entry for each one.

<!-- BEGIN GENERATED: crate-table (tools/gen-crate-index.py) -->
**Foundation**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`broadcast-auth`](broadcast-auth/) | Shared HTTP/RTSP auth: Basic/Digest/Bearer client credentials and a server-side challenge+verify Verifier. | RFC 7235, RFC 7616 / RFC 2617 (Digest), RFC 6750 (Bearer), RFC 2326 §14 | [![crates.io](https://img.shields.io/crates/v/broadcast-auth.svg)](https://crates.io/crates/broadcast-auth) |
| [`broadcast-common`](broadcast-common/) | Shared Parse/Serialize traits, CRC-32/MPEG-2, BCD/time/hex codecs, mux traits, 33-bit clock arithmetic. | ISO/IEC 13818-1 CRC-32 (Annex A); ISO/IEC 23001-7 scheme identity (`cenc`) | [![crates.io](https://img.shields.io/crates/v/broadcast-common.svg)](https://crates.io/crates/broadcast-common) |

**MPEG-2 TS / PES / PS framing**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`dvb-stream`](dvb-stream/) | Async/tokio stream adapters: SI sections and T2-MI events from files, TCP or UDP multicast. | none of its own (adapters over dvb-si / dvb-t2mi); RFC 3550 RTP unwrapping via rtp-packet | [![crates.io](https://img.shields.io/crates/v/dvb-stream.svg)](https://crates.io/crates/dvb-stream) |
| [`mpeg-pes`](mpeg-pes/) | PES depacketization and 33-bit PTS/DTS, with a per-PID PES assembler. | ISO/IEC 13818-1:2021 (ITU-T H.222.0 06/2021) §2.4.3.6-§2.4.3.7 | [![crates.io](https://img.shields.io/crates/v/mpeg-pes.svg)](https://crates.io/crates/mpeg-pes) |
| [`mpeg-ps`](mpeg-ps/) | MPEG-2 Program Stream: pack header (42-bit SCR), system header, program stream map. | ISO/IEC 13818-1:2021 §2.5 | [![crates.io](https://img.shields.io/crates/v/mpeg-ps.svg)](https://crates.io/crates/mpeg-ps) |
| [`mpeg-ts`](mpeg-ts/) | MPEG-2 TS packets, adaptation field, PCR, PSI section reassembly + packetisation, resync. | ITU-T H.222.0 (08/2023) = ISO/IEC 13818-1 §2.4.3, §2.4.4 | [![crates.io](https://img.shields.io/crates/v/mpeg-ts.svg)](https://crates.io/crates/mpeg-ts) |

**DVB service information, physical-layer framing and conformance**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`dvb-bbframe`](dvb-bbframe/) | DVB-S2 / S2X / T2 BBFRAME headers and user-packet extraction (NM and HEM). | ETSI EN 302 307-1 / -2 (DVB-S2 / S2X); EN 302 755 V1.4.1 (DVB-T2, Annex C ISSY) | [![crates.io](https://img.shields.io/crates/v/dvb-bbframe.svg)](https://crates.io/crates/dvb-bbframe) |
| [`dvb-conformance`](dvb-conformance/) | ETSI TR 101 290 transport-stream conformance monitor (priority 1, 2 and 3 indicators). | ETSI TR 101 290 V1.4.1 (2020-06) Tables 5.0a/b/c; partial ISO/IEC 13818-1 T-STD buffer model | [![crates.io](https://img.shields.io/crates/v/dvb-conformance.svg)](https://crates.io/crates/dvb-conformance) |
| [`dvb-si`](dvb-si/) | DVB Service Information + MPEG-2 PSI: every table and descriptor, DSM-CC carousel, Annex A text, SiDemux. | ETSI EN 300 468 V1.20.1 (baseline V1.19.1); ISO/IEC 13818-1 PSI; ISO/IEC 13818-6 + TR 101 202 (carousel); TS 102 006 (SSU); TS 102 809 (AIT); TS 102 323; EN 301 192; NorDig LCN; Chinese AVS2/AVS3/caption descriptors | [![crates.io](https://img.shields.io/crates/v/dvb-si.svg)](https://crates.io/crates/dvb-si) |
| [`dvb-t2mi`](dvb-t2mi/) | DVB-T2 Modulator Interface (T2-MI): all packet types and a feed-and-iterate pump. | ETSI TS 102 773 V1.4.1; EN 302 755 V1.4.1 (L1 signalling) | [![crates.io](https://img.shields.io/crates/v/dvb-t2mi.svg)](https://crates.io/crates/dvb-t2mi) |

**Common Interface, conditional access, scrambling, SimulCrypt**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`dvb-ci`](dvb-ci/) | DVB Common Interface (EN 50221) APDUs, session/transport PDUs, CI Plus extensions, CA_PMT builder. | ETSI EN 50221:1997; TS 101 699 V1.1.1; TS 103 205 V1.4.1 (CI Plus extensions) | [![crates.io](https://img.shields.io/crates/v/dvb-ci.svg)](https://crates.io/crates/dvb-ci) |
| [`dvb-ci-runtime`](dvb-ci-runtime/) | EN 50221 driver runtime: device I/O, TPDU/SPDU poll loop, resource state machines, CAM hot-plug. | ETSI EN 50221:1997 §7-§8; TS 101 699 | [![crates.io](https://img.shields.io/crates/v/dvb-ci-runtime.svg)](https://crates.io/crates/dvb-ci-runtime) |
| [`dvb-csa`](dvb-csa/) | DVB Common Scrambling Algorithm (CSA2) block + stream cipher, with a bitsliced batch path. | DVB-CSA2 (no public normative specification; validated against libdvbcsa known-answer vectors) | [![crates.io](https://img.shields.io/crates/v/dvb-csa.svg)](https://crates.io/crates/dvb-csa) |
| [`dvb-simulcrypt`](dvb-simulcrypt/) | DVB SimulCrypt head-end message framing: generic TLV message plus ECMG<->SCS and EMMG/PDG<->MUX registries. | ETSI TS 103 197 V1.5.1 | [![crates.io](https://img.shields.io/crates/v/dvb-simulcrypt.svg)](https://crates.io/crates/dvb-simulcrypt) |

**Subtitles, captions, teletext, VBI and ancillary data**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`caption-convert`](caption-convert/) | Caption conversion: CEA-608/708 and EBU Teletext to WebVTT/SRT, WebVTT<->SRT. | W3C WebVTT; SRT (de facto); consumes CEA-608/708, EN 300 706 teletext, IMSC | [![crates.io](https://img.shields.io/crates/v/caption-convert.svg)](https://crates.io/crates/caption-convert) |
| [`cc-data`](cc-data/) | Closed-caption cc_data() carriage: typed CEA-608/708 triplets, 608/708 split, optional decode. | ETSI TS 101 154 V2.6.1 Table B.9; ANSI/CTA-608-E; ANSI/CTA-708-E S-2023 (decode feature) | [![crates.io](https://img.shields.io/crates/v/cc-data.svg)](https://crates.io/crates/cc-data) |
| [`dvb-subtitle`](dvb-subtitle/) | DVB bitmap subtitling segments (page/region/CLUT/object/DDS/disparity) and pixel-data sub-blocks. | ETSI EN 300 743 V1.6.1 | [![crates.io](https://img.shields.io/crates/v/dvb-subtitle.svg)](https://crates.io/crates/dvb-subtitle) |
| [`dvb-vbi`](dvb-vbi/) | VBI data in DVB PES: VPS, WSS, closed captioning, EBU/inverted teletext, monochrome samples. | ETSI EN 301 775 V1.2.1 §4; EN 300 472 (teletext carriage) | [![crates.io](https://img.shields.io/crates/v/dvb-vbi.svg)](https://crates.io/crates/dvb-vbi) |
| [`st291`](st291/) | SMPTE ST 291-1 ancillary (ANC) data packets over ST 2038 (TS/PES) and RFC 8331 / ST 2110-40 (RTP). | SMPTE ST 291-1:2011; ST 2038:2021 (`ts` feature); RFC 8331 / ST 2110-40:2023 (`rtp` feature) | [![crates.io](https://img.shields.io/crates/v/st291.svg)](https://crates.io/crates/st291) |
| [`ttml-subtitle`](ttml-subtitle/) | W3C TTML2 / IMSC 1.1 timed-text parser, serializer and profile validator. | W3C TTML2 (REC 2018-11-08); W3C IMSC 1.1 (REC 2018-11-08) | [![crates.io](https://img.shields.io/crates/v/ttml-subtitle.svg)](https://crates.io/crates/ttml-subtitle) |

**Metadata and ad-insertion signalling**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`mp4-emsg`](mp4-emsg/) | ISO BMFF / DASH Event Message Box (emsg) v0 and v1. | ISO/IEC 23009-1 emsg (edition unverified); DASH-IF IOP Part 10 V5.0.0 | [![crates.io](https://img.shields.io/crates/v/mp4-emsg.svg)](https://crates.io/crates/mp4-emsg) |
| [`scte104`](scte104/) | SCTE 104 automation-to-compression messages: single/multiple operation messages and the DPI operations. | ANSI/SCTE 104 2023 | [![crates.io](https://img.shields.io/crates/v/scte104.svg)](https://crates.io/crates/scte104) |
| [`scte35-splice`](scte35-splice/) | SCTE 35 splice_info_section: every splice command and descriptor, segmentation tables, builders. | ANSI/SCTE 35 2023r1; ETSI TS 103 752-1 V1.2.1 (DVB targeted advertising profile) | [![crates.io](https://img.shields.io/crates/v/scte35-splice.svg)](https://crates.io/crates/scte35-splice) |
| [`timed-metadata`](timed-metadata/) | Convert SCTE 35 to/from DASH emsg and to HLS EXT-X-DATERANGE; 33-bit PTS wrap unrolling. | ANSI/SCTE 214-3 §8.3.3 (emsg carriage); RFC 8216 / rfc8216bis §4.4.5.1 (DATERANGE); EN 300 706 V1.2.1 teletext decode (`teletext`) | [![crates.io](https://img.shields.io/crates/v/timed-metadata.svg)](https://crates.io/crates/timed-metadata) |

**Containers and adaptive streaming**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`broadcast-hls`](broadcast-hls/) | HLS (M3U8) playlist syntax: media/master playlists, LL-HLS tags, I-frame variants, CENC/CBCS EXT-X-KEY. | RFC 8216; draft-pantos-hls-rfc8216bis-22 (incl. Appendix D interstitials) | [![crates.io](https://img.shields.io/crates/v/broadcast-hls.svg)](https://crates.io/crates/broadcast-hls) |
| [`container-probe`](container-probe/) | Container-format detection over a byte prefix, with scored evidence and an explicit Ambiguous verdict. | Uses layouts from ISO/IEC 13818-1, ISO/IEC 14496-12, RFC 8794/9559 (EBML/Matroska), SMPTE ST 377-1 and others; implements no standard of its own | [![crates.io](https://img.shields.io/crates/v/container-probe.svg)](https://crates.io/crates/container-probe) |
| [`transmux`](transmux/) | Any-to-any container remux hub: TS/fMP4/PS/WebM/FLV/RTMP in, CMAF/MP4/TS/HLS/DASH/LL-*/Smooth out. | ISO/IEC 14496-12 / -15 / -30, ISO/IEC 23000-19 (CMAF), ISO/IEC 23009-1 (DASH), ISO/IEC 23001-7 (CENC), ISO/IEC 13818-1, RFC 8216, RFC 3550, MS-SSTR, GY/T 420-2025 (AVS2/AVS3/Audio Vivid in ISOBMFF); editions mostly unverified (several ISO texts are paid and not vendored) | [![crates.io](https://img.shields.io/crates/v/transmux.svg)](https://crates.io/crates/transmux) |

**Live transport protocols**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`broadcast-client`](broadcast-client/) | Generic sans-IO client session: auth, redirects, TS-over-HTTP source. | none (implementation-defined policy; requirements from zenith-tv player) | not on crates.io |
| [`buffering-runtime`](buffering-runtime/) | Sans-IO live-TS playout buffer controller: jitter measurement, stall memory, start/pause/resume decisions. | RFC 3550 s6.4.1/A.8 and RFC 5481 (measurement); the sizing policy is implementation-defined | [![crates.io](https://img.shields.io/crates/v/buffering-runtime.svg)](https://crates.io/crates/buffering-runtime) |
| [`hls-runtime`](hls-runtime/) | Sans-IO Low-Latency HLS client and origin engines; optional tokio+reqwest client adapter. | RFC 8216; draft-pantos-hls-rfc8216bis (LL-HLS) | [![crates.io](https://img.shields.io/crates/v/hls-runtime.svg)](https://crates.io/crates/hls-runtime) |
| [`rist-runtime`](rist-runtime/) | RIST Simple Profile RTCP messages (Generic/Range NACK, RTT Echo, compounds) and a sans-IO ARQ engine. | VSF TR-06-1:2020 (RIST Simple Profile); RFC 4585 §6.2.1 | [![crates.io](https://img.shields.io/crates/v/rist-runtime.svg)](https://crates.io/crates/rist-runtime) |
| [`rtcp-packet`](rtcp-packet/) | RTCP SR/RR/SDES/BYE/APP and compound packets. | RFC 3550 §6 | [![crates.io](https://img.shields.io/crates/v/rtcp-packet.svg)](https://crates.io/crates/rtcp-packet) |
| [`rtmp-runtime`](rtmp-runtime/) | Sans-IO RTMP 1.0 ingest (publish) engine: handshake, chunk stream, AMF0 commands, server state machine. | Adobe RTMP 1.0 (Dec 2012); AMF0 | [![crates.io](https://img.shields.io/crates/v/rtmp-runtime.svg)](https://crates.io/crates/rtmp-runtime) |
| [`rtp-packet`](rtp-packet/) | RTP fixed header, CSRC list, generic header extension; optional RFC 8285 extensions and AV3A payload. | RFC 3550 §5.1 / §5.3.1; RFC 8285 (`rfc8285`); T/AI 109.7-2024 §10.1 AV3A-AATF payload (`av3a`) | [![crates.io](https://img.shields.io/crates/v/rtp-packet.svg)](https://crates.io/crates/rtp-packet) |
| [`rtsp-runtime`](rtsp-runtime/) | Sans-IO RTSP 1.0 client + server session engine, interleaved RTP/RTCP, Basic/Digest/Bearer auth. | RFC 2326 (RTSP 1.0) incl. Appendix A state machines, §10.12 interleaving, §12.39 Transport, §14 auth | [![crates.io](https://img.shields.io/crates/v/rtsp-runtime.svg)](https://crates.io/crates/rtsp-runtime) |
| [`srt-runtime`](srt-runtime/) | SRT packet codecs, HSv5 handshake (caller/listener/rendezvous), ARQ, TSBPD, LiveCC/FileCC, optional crypto. | draft-sharabayko-srt-01 | [![crates.io](https://img.shields.io/crates/v/srt-runtime.svg)](https://crates.io/crates/srt-runtime) |
| [`st2022`](st2022/) | SMPTE ST 2022-6 HBRMT (SDI over IP) RTP payload header. | SMPTE ST 2022-6:2012 | [![crates.io](https://img.shields.io/crates/v/st2022.svg)](https://crates.io/crates/st2022) |
| [`tshttp-runtime`](tshttp-runtime/) | Sans-IO TS-over-HTTP pull session: reconnect/backoff, start and stall timers, demuxed events. | none (implementation-defined policy; requirements from the zenith-tv player) | not on crates.io |
| [`webrtc-runtime`](webrtc-runtime/) | Sans-IO WHIP + WHEP HTTP signalling: SDP offer/answer, Trickle ICE, ICE restart, session lifecycle. | RFC 9725 (WHIP); draft-ietf-wish-whep-04; optional ICE/DTLS-SRTP transport (`media`) | [![crates.io](https://img.shields.io/crates/v/webrtc-runtime.svg)](https://crates.io/crates/webrtc-runtime) |

**Servers, runtimes and tools**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`compliance-probe`](compliance-probe/) | Live TR 101 290 + PCR-drift + SCTE 35 cue-sanity probe over a media-plane Trunk, exported as metrics. | ETSI TR 101 290 (via dvb-conformance); SCTE 35 | [![crates.io](https://img.shields.io/crates/v/compliance-probe.svg)](https://crates.io/crates/compliance-probe) |
| [`dvb-tools`](dvb-tools/) | DVB stream analyzer CLI: dump / services / epg / pids / t2mi. | see dvb-si, dvb-t2mi | [![crates.io](https://img.shields.io/crates/v/dvb-tools.svg)](https://crates.io/crates/dvb-tools) |
| [`media-doctor`](media-doctor/) | Media diagnostics: lint-style TS checks, HLS/DASH/fMP4/CMAF validation, optional Prometheus endpoint and CLI. | Checks against ISO/IEC 13818-1, RFC 8216, ISO/IEC 23009-1, SCTE 35 | [![crates.io](https://img.shields.io/crates/v/media-doctor.svg)](https://crates.io/crates/media-doctor) |
| [`media-plane`](media-plane/) | The live-origin spine: dialers/listeners, byte stages, ingest sessions and the Trunk ring hub with DVR retention. | none of its own (architecture layer) | [![crates.io](https://img.shields.io/crates/v/media-plane.svg)](https://crates.io/crates/media-plane) |
| [`multimux`](multimux/) | Multi-input, multi-output just-in-time repackaging HTTP origin (tokio + axum). | Composes the live and container crates (RTSP, RTP, SRT, RTMP, HLS, DASH, Smooth, WHIP/WHEP) | [![crates.io](https://img.shields.io/crates/v/multimux.svg)](https://crates.io/crates/multimux) |
| [`multimux-cli`](multimux-cli/) | The multimux command-line binary: JSON multi-route config or a single-route RTSP quick start. | see multimux | [![crates.io](https://img.shields.io/crates/v/multimux-cli.svg)](https://crates.io/crates/multimux-cli) |
| [`playout-runtime`](playout-runtime/) | Sans-IO linear channel playout: schedule model, transition planning, SCTE 35 splice_insert emission points. | SCTE 35 (via scte35-splice) | [![crates.io](https://img.shields.io/crates/v/playout-runtime.svg)](https://crates.io/crates/playout-runtime) |
| [`ssai-runtime`](ssai-runtime/) | Sans-IO SCTE 35 server-side ad insertion core with HLS Interstitials rendering. | SCTE 35; draft-pantos-hls-rfc8216bis Appendix D (interstitials) | [![crates.io](https://img.shields.io/crates/v/ssai-runtime.svg)](https://crates.io/crates/ssai-runtime) |
| [`ts-fix`](ts-fix/) | MPEG-2 TS repair/remux engine and CLI: continuity, PID filter, PAT/PMT regen, stuffing, PCR restamp. | ISO/IEC 13818-1 | [![crates.io](https://img.shields.io/crates/v/ts-fix.svg)](https://crates.io/crates/ts-fix) |

**Professional media (SMPTE / AES / Dolby)**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`rdd29`](rdd29/) | SMPTE RDD 29 Dolby Atmos bitstream: frame/element framing plus bed/object metadata. | SMPTE RDD 29:2019 | [![crates.io](https://img.shields.io/crates/v/rdd29.svg)](https://crates.io/crates/rdd29) |
| [`st12-1`](st12-1/) | SMPTE ST 12-1 Linear Timecode: the 80-bit LTC codeword. | SMPTE ST 12-1:2014 §9 | [![crates.io](https://img.shields.io/crates/v/st12-1.svg)](https://crates.io/crates/st12-1) |
| [`st337`](st337/) | SMPTE ST 337 non-PCM audio/data burst framing over AES3. | SMPTE ST 337:2015 | [![crates.io](https://img.shields.io/crates/v/st337.svg)](https://crates.io/crates/st337) |
| [`st377-1`](st377-1/) | SMPTE ST 377-1 MXF: KLV, partition/primer packs, structural metadata, OP1a packages, random index pack. | SMPTE ST 377-1:2019; ST 336:2017 (KLV); ST 378M-2004 (OP1a) | [![crates.io](https://img.shields.io/crates/v/st377-1.svg)](https://crates.io/crates/st377-1) |

**IP and multicast delivery over broadcast**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`atsc3`](atsc3/) | ARCHIVED, never published: ATSC 3.0 LLS envelope + Service List Table parser. | ATSC A/331 §6.2-§6.3 | archived, never published |
| [`atsc3-route`](atsc3-route/) | ARCHIVED (maintenance only): ATSC 3.0 ROUTE header-extension constants, FEC payload IDs, codepoint table. | ATSC A/331 Annex A; RFC 6330 (RaptorQ payload ID) | [![crates.io](https://img.shields.io/crates/v/atsc3-route.svg)](https://crates.io/crates/atsc3-route) archived |
| [`dvb-mabr`](dvb-mabr/) | DVB multicast ABR session-configuration XML parser/serializer. | ETSI TS 103 769 V1.2.1 clause 10 | [![crates.io](https://img.shields.io/crates/v/dvb-mabr.svg)](https://crates.io/crates/dvb-mabr) |
| [`rmt-flute`](rmt-flute/) | ALC/LCT/FLUTE/NORM multicast object-delivery wire formats. | RFC 5651 (LCT); RFC 5775 (ALC); RFC 6726 (FLUTE); RFC 5740 (NORM); RFC 5052 (FEC building block) | [![crates.io](https://img.shields.io/crates/v/rmt-flute.svg)](https://crates.io/crates/rmt-flute) |
| [`ule`](ule/) | ULE: IP over MPEG-2 TS (SNDU framing, extension headers, TS de-fragmentation). | RFC 4326; RFC 5163 | [![crates.io](https://img.shields.io/crates/v/ule.svg)](https://crates.io/crates/ule) |

**Measurement**

| Crate | What it does | Headline standards | Status |
|---|---|---|---|
| [`broadcast-loudness`](broadcast-loudness/) | EBU R 128 / ITU-R BS.1770-5 loudness: integrated, momentary, short-term, LRA, true peak. | ITU-R BS.1770-5 (11/2023); EBU R 128, Tech 3341, Tech 3342 (all November 2023); GY/T 282-2014, GY/T 377-2023, GY/T 379-2023 target profiles | [![crates.io](https://img.shields.io/crates/v/broadcast-loudness.svg)](https://crates.io/crates/broadcast-loudness) |

Plus `test-alloc`, `test-bounded`, `test-fixtures`: `publish = false` test helpers, see [docs/CRATES.md](docs/CRATES.md).
<!-- END GENERATED: crate-table -->

Outside the workspace: Python bindings [`dvb-si-py`](bindings/python/) and
[`transmux-py`](bindings/transmux-py/), and the docs-only spec directories
[`cpix/`](cpix/) and [`st2110/`](st2110/). See [docs/CRATES.md](docs/CRATES.md#outside-the-workspace).

## Getting started

Add the crate you need; each is independent and pulls in only its own
workspace dependencies:

```toml
[dependencies]
dvb-si = "12"
```

Read SDT service names from a transport stream:

```rust
use dvb_si::demux::SiDemux;
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::tables::AnyTableSection;

let mut demux = SiDemux::builder().build();
for packet in ts_packets {                       // each aligned 188-byte packet
    for event in demux.feed(&packet) {           // only sections that changed
        if let Ok(AnyTableSection::SdtSection(sdt)) = event.table_section() {
            for service in &sdt.services {
                for item in service.descriptors.iter().flatten() {
                    if let AnyDescriptor::Service(svc) = item {
                        println!("{}", svc.service_name.decode()); // Annex A text to UTF-8
                    }
                }
            }
        }
    }
}
```

Or from the command line, without writing code:

```console
$ cargo run -p dvb-tools -- dump dvb-si/tests/fixtures/m6-single.ts
pid=0x0000 PROGRAM_ASSOCIATION v0 sn=0
pid=0x0064 PROGRAM_MAP v1 sn=0
-- packets=1264 sections=47 emitted=3 suppressed=44 crc_failures=0 malformed=0
```

Each crate's README and docs.rs front page carry a compiling quick start for
that crate.

### Toolchain and `no_std`

| | |
|---|---|
| **MSRV** | 1.95.0 for every member (`rust-version.workspace`, pinned in `rust-toolchain.toml`); always build with `--locked` |
| **Edition** | 2024 |
| **`no_std`** | Most parser and builder crates are `#![no_std]` + `alloc` with default features off. Only the crates CI builds for `thumbv7em-none-eabi` count as proven; the `no_std (CI)` column in [docs/CRATES.md](docs/CRATES.md#overview) lists them. Build one crate at a time (`cargo build -p dvb-si --no-default-features --locked`): a workspace-wide build unifies features and can hide a `std` dependency. |
| **Versions** | Five DVB crates (`dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`) release in lockstep from one `v<version>` tag. Every other crate, including `broadcast-common`, versions independently. |

## Why trust these crates

- **Grounded in the standard, clause by clause.** Spec syntax tables are
  transcribed into reviewable Markdown under each crate's `docs/` directory,
  and every module doc cites the spec, edition and clause it implements. There
  are no magic numbers: every hex literal outside tests is a named constant or
  enum.
- **Symmetric.** Every parser has a serializer. Parse then serialize is
  byte-identical, and serialize then parse is equal; that round trip is a hard
  invariant enforced by tests, so you can build PSI tables, SCTE 35 cues or
  EN 50221 APDUs, not just read them.
- **Decoded, not just typed.** Spec-enumerated codes are enums with their
  spec names (`name()` plus `Display`), so consumers do not re-implement lookup
  tables.
- **Tested on real data.** Fixture tests run against real broadcast captures
  and reference streams; parsers are fuzzed.
- **Honest about gaps.** Partial coverage is documented as partial, in each
  crate's README and in [docs/STANDARDS.md](docs/STANDARDS.md).

## Contributing

- [CLAUDE.md](CLAUDE.md) — the project guide: commands, CI gates, workflow
  (GitHub issue, branch, PR), architecture and conventions.
- [docs/CRATE-ACCEPTANCE.md](docs/CRATE-ACCEPTANCE.md) — the bar every new
  crate has to clear.
- [docs/DOC-STANDARD.md](docs/DOC-STANDARD.md) — how a crate README and
  crate-root documentation must be structured.
- [docs/RELEASE-DOCS.md](docs/RELEASE-DOCS.md) and
  [docs/RELEASE-AUDIT.md](docs/RELEASE-AUDIT.md) — what every release must
  produce and check.
- [docs/extending.md](docs/extending.md) — adding a parser crate that plugs
  into the existing dispatch.
- [docs/README.md](docs/README.md) — index of everything under `docs/`.

Releases are tag-driven and published by CI only.

## Licence

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Contributions are accepted under the same dual licence.
