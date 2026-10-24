---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 1 Introduction (Tables 1-3)
pdf_pages: 22-25
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch01-introduction.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 1 Introduction (notes)

PDF p.22 = printed p.1. Verbatim: private file above.

## What the clause says

- The document defines only the interoperability points (IOPs) of this version; earlier versions [1], [2] are to be deprecated in favour of the IOPs and extensions here.
- History: v1 basic DASH-AVC/264; v2 added HD (to 1080p) and multichannel audio; v3 added two AVC IOPs (simple, main) plus HEVC counterparts, the "main" one adding client segment parsing for live; v4 added next-generation audio and UHD/HDR.
- All v1 IOPs are covered by v2; all v2 extensions carry into v3 unchanged.
- Test tools: DASH-IF conformance software [32], test assets [31], identifier registry [33]. Identifiers in italics are under discussion; technologies without test material are candidates only.

## Table 1 — interoperability points (PDF pp.22-23)

Identifiers are under `http://dashif.org/guidelines/`.

| IOP | Identifier suffix | Version | Where |
|---|---|---|---|
| DASH-AVC/264 | `dash264` | 1.0 | [2] 6.3 |
| DASH-AVC/264 SD | `dash264#sd` | 1.0 | [2] 7.3 |
| DASH-AVC/264 HD | `dash264#hd` | 2.0 | [2] 8.3 |
| DASH-AVC/264 main | `dash264main` | 3.0 | 8.2 |
| DASH-AVC/264 high | `dash264high` | 3.0 | 8.3 |
| IOP simple | `dash-if-simple` | 3.0 | 8.4 |
| IOP main | `dash-if-main` | 3.0 | 8.5 |
| IOP on-demand | `dash-if-ondemand` | 4.3 | 3.10.3 |
| IOP mixed on-demand | `dash-if-mixed` | 4.3 | 3.10.4 |

## Table 2 — extensions (PDF pp.23-25)

Base `http://dashif.org/guidelines/`; version and clause in the last two columns.

| Extension | Suffix | Ver | Clause |
|---|---|---|---|
| multichannel E-AC-3 | `dashif#ec-3` | 2.0 | 9.4.2.3 |
| multichannel Dolby TrueHD | `dashif#mlpa` | 2.0 | 9.4.2.3 |
| multichannel AC-4 | `dashif#ac-4` | 3.1 | 9.4.2.3 |
| DTS Digital Surround | `dashif#dtsc` | 2.0 | 9.4.3.3 |
| DTS-HD HR / Master Audio | `dashif#dtsh` | 2.0 | 9.4.3.3 |
| DTS Express | `dashif#dtse` | 2.0 | 9.4.3.3 |
| DTS-HD Lossless (no core) | `dashif#dtsl` | 2.0 | 9.4.3.3 |
| MPEG Surround | `dashif#mps` | 2.0 | 9.4.4.3 |
| HE-AACv2 level 4 | `dashif#heaac-mc51` | 2.0 | 9.4.5.3 |
| HE-AACv2 level 6 | `dashif#heaac-mc71` | 2.0 | 9.4.5.3 |
| MPEG-H 3D Audio | `dashif#mpeg-h-3da` | 4.2 | 9.4.6.3 |
| USAC | `dashif#cxha` | 4.3 | 9.4.7.3 |
| UHD HEVC 4k | `dash-if-uhd#4k` | 4.0 | 10.2 |
| HEVC HDR PQ10 | `dash-if-uhd#hdr-pq10` | 4.0 | 10.3 |
| UHD Dual-Stream (Dolby Vision) | `dash-if-uhd#hdr-pq10` (as printed) | 4.1 | 10.4 |
| VP9 HD | `dashif#vp9` | 4.1 | 11.3.1 |
| VP9 UHD | `dash-if-uhd#vp9` | 4.1 | 11.3.2 |
| VP9 HDR | `dashif#vp9-hdr` and `dash-if-uhd#vp9-hdr` | 4.1 | 11.3.3 |

> **Notes:** (1) The Dual-Stream row repeats the identifier of the HEVC HDR PQ10 row (p.24), almost certainly a copy slip, kept as printed. (2) The reference column of Table 1 is one character wide in the PDF, so its values are printed vertically. (3) Identifiers wrap across lines in Table 2.

## Table 3 — other identifiers (PDF p.25)

| Identifier | Meaning | Type | Clause |
|---|---|---|---|
| `http://dashif.org/identifiers/vast30` | event signalling VAST 3.0 | event | 5.6 |
| `http://dashif.org/guidelines/trickmode` | trick-mode Adaptation Set | functionality | 3.2.9 |
| `http://dashif.org/guidelines/clearKey` | namespace for Laurl in W3C Clear Key | namespace | 7.6.2.4 |
| `e2719d58-a985-b3c9-781a-b030af78d30e` | UUID for W3C Clear Key in DASH | content protection | 7.6.2.4 |
| `http://dashif.org/guidelines/last-segment-number` | last segment number | functionality | 4.4.3.6 |
| `http://dashif.org/guidelines/thumbnail_tile` | thumbnail tile Adaptation Set | functionality | 6.2.6 |
