---
title: "Clauses 10.10-10.11 MPD processing and HTTP support (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.10-10.11
pdf_pages: 94-95
printed_pages: 94-95
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-6-mpd-processing-http.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clauses 10.10-10.11 MPD processing and HTTP support (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

## 10.10 MPD processing (p.94-95)

- Players shall support the 2014 profile, may support 2017, and shall play the content of the profile-specific MPD (23009-1 clause 8.1).
- Several AdaptationSets of one component type: by default select one with Role `main` (`urn:mpeg:dash:role:2011`) unless the player understands other signalling or an application chooses otherwise. A lone audio set with no Role counts as `main`. A lone audio set whose Role is not `main` shall not be chosen unless the player understands the signalling or an application asks.
- Several `main` sets: highest `@selectionPriority`; ties are the player's choice (possibly random).
- Players shall ignore Representations whose `FramePacking` scheme/value they do not recognise (23009-1 5.8.4.6 note); shall ignore a parent element whose `EssentialProperty` child is unrecognised (5.8.4.8); shall process `EssentialProperty`/`SupplementalProperty` on the MPD element.
- Dynamic MPDs: players shall download and use updated MPDs within the changes allowed by clause 4.8.

## 10.11 HTTP support (p.95)

Shall: HTTP/1.1 (RFC 7230) with cookies (RFC 6265); byte-range requests; gzip content coding (RFC 7231 3.1.2, RFC 7230 4); follow redirects 301/302/307 with `Location` (RFC 7231 6.4), at least three in a row, a cached redirect affecting only its own URL; use the `MPD.Location` URL, not the original URL, for MPD updates; keep-alive TCP connections for further segments from the same server; SSL session caching where SSL is supported.
Should: follow the HTTP status/error guidance of 23009-1 A.7; use a TCP connect timeout of about 1-3 s (inter-byte timeouts of similar size can help but must not misfire on throttled or idle connections).

## Notes on the source

- Typo "contstraints" (10.10); a URN broke across lines in the first paragraph (read as `dvb-dash:2014`).
