---
source: private/specs/dashif_iop_v4.3_guidelines.pdf (DASH-IF "Guidelines for Implementation: DASH-IF Interoperability Points", v4.3, 15 Nov 2018)
clauses: Scope, Disclaimer, Acronyms, References
pdf_pages: 3-4, 13-21
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch00-front-matter.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — front matter (notes)

Original-wording notes. Verbatim text: see the private file above.

## Scope and disclaimer (PDF pp.3-4)

- Scope: guidelines for over-the-top delivery of H.264/AVC and H.265/HEVC video, live and on-demand; use cases named are on-demand, live, ad insertion, trick modes, seek preview, content protection, subtitling; extensions cover multichannel/next-generation audio, UHD and further codecs.
- v4.3 is stated to be an add-on to v4.2; a restructured or multipart successor was announced for spring 2019.
- Disclaimer: standard IPR/no-warranty text; technologies without published test/conformance material are only "candidate" and may be dropped.

> **Note:** page footers are inconsistent: front matter says V4.2, printed p.1 says v4.1, printed p.2 onward says v4.3. From the Introduction on, printed page = PDF page - 21. Contents (PDF 5-11) and the figure/table lists (11-13) are navigation only and are not covered.

## Acronyms (PDF pp.13-15)

The document defers general terms to ISO/IEC 23009-1 and adds a list of about 80 abbreviations. Those a client implementer meets most: AST (availability start time), SAST/SAET (segment availability start/end time), MBT (minimum buffer time), MUP (minimum update period), PT (presentation time), PTO (presentation time offset), EPT (earliest presentation time), SPD (suggested presentation delay), TSB (time shift buffer depth), SAP (stream access point), KID (common key identifier), IOP (interoperability point), TT/TTML (timed text), UTC. Oddities: HDR and HEVC are each listed twice (the second HEVC without a hyphen), and UTC is expanded as "Universal Time Clock". Full list: private verbatim file.

## References (PDF pp.16-21)

91 numbered entries, [51] and [70] void. Entries the client-relevant clauses cite most:

| Ref | Document |
|---|---|
| [4] | ISO/IEC 23009-1:2014 with Cor 1-2 and Amd 1-3 (Amd 4/Cor 3/Amd 5 cited as in preparation) |
| [6] | ISO/IEC 23009-3:2014 (implementation guidelines) |
| [7] | ISO/IEC 14496-12:2015 (ISO BMFF) |
| [8], [19] | H.264 (04/2017), H.265 (02/2018) |
| [10] | RFC 6381 (codecs/profiles parameters) |
| [21]-[25] | RFC 7230-7234 (HTTP/1.1) |
| [29] | ISO/IEC 14496-30 (timed text in ISOBMFF) |
| [30] | ISO/IEC 23001-7:2016 (common encryption) |
| [39] | DTS 9302K62400 (DTS in DASH) |
| [40] | RFC 5905 (NTP) |
| [41] | RFC 6265 (cookies) |
| [42], [76] | ETSI TS 103 285 v1.1.1 (DVB-DASH; cited at v1.1.1 here) |
| [44] | RFC 2119 |
| [49], [72] | ISO/IEC 23001-8 (2016, 2013) code points |
| [54]-[57] | SCTE 35, 67, 214-1, 214-3 |
| [61] | IMSC1 draft (3 Aug 2015) |
| [65] | RFC 5246 (TLS 1.2) |
| [66] | RFC 4337 (MP4 media types) |
| [69] | W3C EME |

Reference-list typography (informative entries in grey Arial: [5], [6], [13], [15], [18], [27], [28], [44], [75], [77]-[79]; none visibly italic) is noted, not reproduced. Quirks: [9] is dated "2017:2015"; [34] and [87] repeat their URL; [81] duplicates [48]. Remaining entries: private verbatim file.
