---
title: "Annex A (informative) Summary of fixes (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: Annex A (A.1, A.2)
pdf_pages: 130-132
printed_pages: 130-132
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/annex-a-fixes.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Annex A (informative) Summary of fixes (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

The annex is a change log (Tables A.1 and A.2; columns: tracker Ref., summary, impacted clauses, severity, previously published Y/blank/N). Only the Player-relevant themes and ids are noted; the full rows are in the private verbatim file.

## Table A.1 (2014 profile fixes, pp.130-132) - player-relevant groups

| Theme | Tracker refs | Clauses |
|---|---|---|
| Live playback, MPD update, availability | 2179 (start position vs suggestedPresentationDelay), 2181 (seeking forward in live), 2469 (minimumUpdatePeriod behaviour), 2727 (low-latency segment timeline), 2817 (time-shift buffer, unavailable segments), 2842 (dynamic to static), 2715 (MPD update requirements), 1331 ("one hour" corrected to "one year" for availabilityStartTime) | 10.9.2, 9.1.4, 10.9.5, 10.9.4.4/11.18.2, 10.9.6, 10.9.7/11.9 [sic], 4.8, 10.9.4.1 |
| UTC timing | 1407, 1408, 2940 (UTCTiming reference, one wrong URN, http-head guidance) | 2.1, 4.7.2, 4.7.3 |
| BaseURL / resilience | 1400, 1404, 2479 (more error cases), 3001 (BaseURL switching example) | 4.2.4-4.2.5, 10.8.2.x, 10.8.5 |
| Reporting | 1405, 1406, 1427, 1428, 1429, 1431, 2493, 2494, 2495, 2503 | 10.12.x, Annex E [sic] |
| Events | 1330, 2468, 2492, 2731, 2745, 2746, 2836 | 9.1.x, 11.10 |
| Periods and continuity | 2534, 2935, 3012 | 10.5.2.x |
| MPD selection / compatibility | 1950, 1952, 2860, 2514, 2473, 2456, 2489 | 6.1, 10.10, 10.19, 10.17, 4.3, 10.2 |
| Trick mode, switching, resolution | 2470, 2304, 2998 | 10.6.0, 10.4, 10.3 |
| Low latency | 2500, 2501, 2502, 2526, 2702, 2934, 2939 | Annex E, 10.20.x, 11.18 |
| Codecs, colour, VVC/AVS3 | 1442, 2012, 2327, 2398-2400, 2932, 2997 (VVC and AVS3 text, High), 3040, 3047 (AVS3 matrix coefficient value 8 not 9; PQ10 transfer value 12), 3048, 3053, 3095 | 5.x, 10.3 |
| Subtitles, fonts, misc | 2395, 2855, 2829, 3044 (example 7.3 gets contentType="text"), 2547, 2747, 2410, 2531, 2708, 2699, 2285 (DTDs prohibited in an MPD), 2295, 1425, 1426, 1440 (xlink onLoad circular resolution), 2710, 2725 | 7.x, 6.x, 4.x, 2.1, 3.1 |

## Table A.2 (2017 profile fixes, p.132)

2436 (preselection wording, 6.7.5), 2472 and 2498 (AC-4 language attribute/AC-4 reference, 6.3.2.6), 2476 (DTS-UHD added across 2.x, 3.2, 6.4, 6.6.2, 6.7.x, 11.3.3, 11.7.1, Annex B, C.3), 2496 and 2528 (NGA example MPD fixes, Annex B). All low severity.

## Notes on the source

- Printed oddities: Ref 2997 appears twice (VVC and AVS3); some rows cite clauses that do not exist (10.21, 11.20.4, 7.2.13, "Annex E" for reporting); 2842 cites 11.9 where 11.19 is meant; "edtition" and "Resoluton" typos; blank Severity/Published cells for several rows.
