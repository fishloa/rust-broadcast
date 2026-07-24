# rust-broadcast roadmap — media-server expansion

Sequenced plan for the backlog filed 2026-07-24 (#736–#760). Ordered **reuse-first / spec-in-hand** before new-spec crates. Almost every spec is free (ETSI, SMPTE-now-free, IETF, Microsoft, ATSC, AMWA, W3C, DASH-IF); only ISO/IEC is paid and already vendored.

## Crate placement — new crate vs addon to an existing one

| # | Item | Home | New crate? |
|---|------|------|-----------|
| #760 | HLS-pull TS-segment ingest | `ll-hls-runtime` (client demux) + `multimux` (source) | addon |
| #738 | RTMP ingest | **`rtmp-runtime`** (sans-IO session: handshake/chunk-stream/AMF — mirrors `rtsp-runtime`) + `multimux` source; `transmux` keeps the FLV/message→Media demux | **NEW** `rtmp-runtime` |
| #739 | SRT ingest | `srt-runtime` (receiver adapter) + `multimux` source | addon |
| #758 | DASH-pull ingest | `transmux::dash` (add MPD **parse**, inverse of the packager) + `multimux` source | addon |
| #759 | Smooth-pull ingest | `transmux::smooth` (add a manifest+fragment **reader**, inverse of `SmoothPackager`) + `multimux` source | addon |
| #742 | Smooth output | `multimux` (Output over `transmux::SmoothPackager`) | addon |
| #744 | RTMP/RTSP/SRT re-egress | `multimux` output (via `transmux` RTMP mux / `rtsp-runtime` / `srt-runtime`) | addon |
| #745 | DRM key-server | **`cpix`** (DASH-IF CPIX XML key-exchange doc parse/build) + `transmux`/`multimux` PSSH/license integration (CENC already present) | **NEW** `cpix` |
| #746 | DVR / recording | `multimux` (durable MediaStore backing + DVR window) | addon |
| #747 | Signed-URL / token gate | `broadcast-auth` (signed-URL scheme) + `multimux` | addon |
| #748 | Linear playout / scheduler | `multimux` (scheduler; reuses `scte35-splice`/`scte104`) | addon |
| #749 | Dynamic config API / hot-reload | `multimux` (control plane) | addon |
| #736 | dvb-conformance P3-remainder | `dvb-conformance` | addon |
| #737 | dvb-conformance T-STD probe | `dvb-conformance` (new module; big) | addon |
| #753 | TTML / IMSC subtitles | **`ttml`** (W3C TTML2 + SMPTE ST 2052 IMSC) | **NEW** `ttml` |
| #755 | DVB-MABR | **`dvb-mabr`** (ETSI TS 103 769) | **NEW** `dvb-mabr` |
| #754 | MXF OP1a / AS-11 | `st377-1` (extend the MXF core) | addon |
| #751 | SMPTE ST 2110-20/30/21 | **`st2110`** (video/audio/timing over IP; sibling of the ST 2110-40 planned in `st291`) | **NEW** `st2110` |
| #752 | SMPTE ST 2022-6/7 | **`st2022`** (SDI-over-IP + hitless redundancy) | **NEW** `st2022` |
| #750 | ATSC 3.0 | **`atsc3`** (A/331 ROUTE+MMT + A/321 bootstrap) | **NEW** `atsc3` |
| #740/#743 | WHIP / WHEP | **`webrtc-runtime`** (ICE/DTLS-SRTP/RTP + WHIP/WHEP) + `multimux` in/out | **NEW** `webrtc-runtime` (biggest) |
| #741 | RIST | **`rist`** (VSF TR-06; RTP + retransmit/FEC) + `multimux` source | **NEW** `rist` |
| #756 | HLS/DASH manifest validators | `media-doctor` | addon |
| #757 | Loudness (BS.1770/R128/A85) | **`loudness`** (needs decoded PCM — design boundary) | **NEW** `loudness` |

**New crates (~10):** `rtmp-runtime`, `cpix`, `ttml`, `dvb-mabr`, `st2110`, `st2022`, `atsc3`, `webrtc-runtime`, `rist`, `loudness`. Each clears `docs/CRATE-ACCEPTANCE.md` + gets its own `release-<crate>.yml` lane.

## Waves (execution order)

- **Wave 0 — legacy-IPTV ingest** (reuse existing code): #760 → #738 → #739 → #758 → #759.
- **Wave 1 — egress**: #742, #744.
- **Wave 2 — commercial**: #745, #747, #749, #746, #748.
- **Wave 3 — dvb-conformance**: #736, #737.
- **Wave 4 — standards crates**: #753 → #755 → #754 → #751 → #752 → #750.
- **Wave 5 — big subsystems + QC**: #740+#743, #741, #756, #757.

## Spec acquisition + "excellent md" transcription — task list

Per project discipline: **before** implementing an item, its spec is collected from the source below, rendered (not `pdftotext`), and transcribed to **excellent md** in the target crate's `docs/`; commit that + a real fixture + the parse/round-trip gate BEFORE delegating the code (as done for TR 101 290 → `dvb-conformance/docs/tr_101_290.md`). ISO/IEC copies: `private/` submodule + the free ossrs collection.

| Spec | For | Source (all free unless noted) | Target docs | Status |
|------|-----|-------------------------------|-------------|--------|
| RFC 8216 / 8216bis (HLS) | #760, #756 | IETF datatracker | (have — cited) | ✅ in hand |
| ISO/IEC 13818-1 TS | #760, #737 | vendored `private/` | (have) | ✅ in hand |
| Adobe RTMP 1.0 | #738 | free (Adobe open spec) | `rtmp-runtime/docs/` | ☐ collect + transcribe |
| SRT protocol | #739 | Haivision (GitHub, free) | `srt-runtime/docs/` (or have) | ☐ verify |
| ISO/IEC 23009-1 (DASH MPD) | #758, #756 | schema public; `private/`/ossrs | `transmux/docs/` | ☐ transcribe MPD parse |
| `[MS-SSTR]` (Smooth) | #759, #742 | learn.microsoft.com (free) | `transmux/docs/` | ☐ collect + transcribe |
| DASH-IF CPIX + AWS SPEKE | #745 | dashif.org / AWS docs (free) | `cpix/docs/` | ☐ collect + transcribe |
| ISO/IEC 23001-7 (CENC) | #745 | vendored `private/` | (have) | ✅ in hand |
| ETSI TR 101 211 (_other rates) | #736 | etsi.org (free) | `dvb-conformance/docs/` | ☐ collect + transcribe |
| W3C TTML2 + SMPTE ST 2052 IMSC | #753 | w3.org / pub.smpte.org (free) | `ttml/docs/` | ☐ collect + transcribe |
| ETSI TS 103 769 (DVB-MABR) | #755 | etsi.org (free) | `dvb-mabr/docs/` | ☐ collect + transcribe |
| SMPTE ST 378 (OP1a) + AMWA AS-11 | #754 | pub.smpte.org / amwa.tv (free) | `st377-1/docs/` | ☐ collect + transcribe |
| SMPTE ST 2110-20/30/21 + RFC 4175 | #751 | pub.smpte.org / IETF (free) | `st2110/docs/` | ☐ collect + transcribe |
| SMPTE ST 2022-6/7 | #752 | pub.smpte.org (free) | `st2022/docs/` | ☐ collect + transcribe |
| ATSC A/331 + A/321 | #750 | atsc.org (free) | `atsc3/docs/` | ☐ collect + transcribe |
| IETF WHIP/WHEP drafts + WebRTC RFCs | #740, #743 | datatracker (free) | `webrtc-runtime/docs/` | ☐ collect + transcribe |
| VSF TR-06-1/-2 (RIST) | #741 | vsf.tv (free) | `rist/docs/` | ☐ collect + transcribe |
| ITU-R BS.1770 + EBU R128 + ATSC A/85 | #757 | itu.int / ebu.ch / atsc.org (free) | `loudness/docs/` | ☐ collect + transcribe |

Spec-prep is dispatched **per wave** (not all up front) so transcriptions stay fresh against the code that cites them. Wave 0 needs only the RTMP transcription (#738); #760/#739/#758/#759 reuse specs already in hand.
