---
source: private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
document: ETSI TS 103 285 V1.4.1 (2023-09), DVB MPEG-DASH profile for ISO BMFF based DVB services over IP
clauses: modal verbs, 1 Scope, 2 References, 3 Definitions/abbreviations
pdf_pages: 11-19 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch01-03-front-scope-refs-defs.md
verified_against_render: true
notes_original_wording: true
---

# TS 103 285 V1.4.1 - front matter and clauses 1-3 (notes)

Original-wording notes. Verbatim, page-verified text lives in the private file named in the front matter.

## Modal verbs (p. 11)
Shall / should / may / need not / will / can follow the ETSI Drafting Rules, clause 3.2. "must" is not used
except inside direct citations. Everything below uses these strengths as the spec does.

## 1 Scope (p. 12)
- The document defines HTTP adaptive-streaming delivery of TV content on top of MPEG-DASH, adding constraints and
  picking DVB-toolbox codecs. Covered: HDTV, UHDTV/UHDTV2, HDR, HFR, Next Generation Audio (NGA), and low-latency
  delivery plus low-latency presentation.
- Normative XML schemas come as a separate zip (`ts_103285v010401p0.zip`); schema text printed in the PDF is
  informative only. The zip is not available in this repository (gap).

## 2 References (pp. 12-14) - identifiers only
Normative [1]-[50] (some Void: [25], [30], [36]) and informative [i.1]-[i.10]. Ones a client implementer needs:

| Ref | Document |
|---|---|
| [1] | ISO/IEC 23009-1:2020 (CSA edition) |
| [50] | ISO/IEC 23009-1 (undated; the body cites both [1] and [50]) |
| [2] | ISO/IEC TR 23009-3 (implementation guidelines) |
| [3] | ETSI TS 101 154 (video/audio coding toolbox) |
| [4] / [6] | ISO/IEC 14496-15 / 14496-12 |
| [5] | IETF RFC 6381 (codecs/profiles parameters) |
| [7] | ISO/IEC 23001-7:2016 (common encryption) |
| [13] / [14] | EBU Tech 3380 (EBU-TT-D) / 3381 (EBU-TT-D in ISO BMFF) |
| [17] / [18] | RFC 7230 / RFC 6265 (HTTP/1.1, cookies) |
| [19] / [20] / [22] | ISO/IEC 14496-30 / 14496-22 (OFF) / W3C WOFF 1.0 |
| [23] / [33] | ETSI TS 103 190-1 / -2 (AC-4) |
| [28] | RFC 5234 (ABNF) |
| [34] / [37] | ISO/IEC 23008-3 (3D audio) / 23003-3 (USAC) |
| [38] / [39] / [21] | ITU-R BT.2020 / BT.2100 / BT.709 |
| [40] | ISO/IEC 23000-19 (CMAF) |
| [41] | ETSI TS 102 822-3-1 (TV-Anytime, AudioPurposeCS) |
| [42] | ETSI TS 103 491 (DTS-UHD) |
| [44] / [45] | AVS T/AI 109.2-2021 (video) / 109.6-2022 (formats) |
| [48] / [49] | ISO/IEC 23091-2 (2021) / 23091-3 (2018) code points |
| [i.1] | DASH-IF IOP v3.0 (informative) |

Note: [1] pins 23009-1:2020 while this workspace transcribes the 2022 5th edition
(`transmux/docs/README.md`, `-ids.md`); no clause conflict was checked.

## 3 Definitions (pp. 15-19)
- 3.1 Terms: standard DASH vocabulary (adaptation set, representation, period, segment kinds, SAP, MPD, UTC
  "wall-clock time", event/event stream, remote element entity, trick mode, HTTP-URL = http or https scheme only).
  DVB-specific terms: **audio preselection** (components decoded together), **audio bundle**, **main / auxiliary NGA
  stream**, **default audio preselection**, **SRSP** (single representation, single preselection), **SRMP**
  (single representation, multiple preselections), **MRMP** (multiple representations, multiple preselections),
  **low latency content / low latency player**, **available segment** (reachable by GET with a 2xx answer).
- 3.2 Symbols: void.
- 3.3 Abbreviations: ordinary expansions (AID, CENC, CMAF, HLG, PQ, MHAS, NGA, SRSP/SRMP/MRMP, VVC, WOFF ...);
  nothing normative. Full list in the private verbatim file.
