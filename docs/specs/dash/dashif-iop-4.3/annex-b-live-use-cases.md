---
source: DASH-IF Interoperability Points v4.3 (15 Nov 2018), private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: "Annex B Live Services - Use Cases and Architecture (B.1-B.5)"
pdf_pages: "230-235"
printed_pages: "annex pp.2-7"
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/annex-b-live-use-cases.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 Annex B - Live use cases and typical problems (notes)

Informative annex; own-words summary, exact text in the private file. "p." = PDF page.

## B.1 Baseline use cases (p.230)

| # | Case | Key facts |
|---|---|---|
| 1 | live content offered on demand | separate presentation |
| 2 | scheduled, known duration, at live edge | ~30 min, all segment URLs known, short timeshift buffer |
| 3 | as 2 plus long time-shift buffer | content not accessible before schedule, then kept ~24 h |
| 4 | scheduled live, known duration, unknown URLs | e.g. ads inserted on the fly |
| 5 | 24/7 live | started long ago, ad breaks/operational updates with ~30 s warning, unknown duration, URLs and media components |
| 6 | approximate duration known | starts at a set time; duration finalised or extended by MPD update |

## B.2-B.3 Architecture (pp.231-232)

Fig. 27 (redundant live DASH with unicast): MPEG-2 TS feed -> multi-bitrate encoder (redundant) -> segmenter + MPD generator (redundant) -> origin servers -> edge servers -> DASH client + media player over HTTP. Encoder usually locks to the TS system clock. CDN may push segments by IP multicast or edge may fetch from parent over HTTP. Ads may interrupt generation and be personalised. Fig. 28 (MBMS): segmenter feeds the BM-SC, which sends one quality over FLUTE to the MBMS UE; a local HTTP cache exposes segments to the DASH client; fall back to unicast HTTP if MBMS reception fails; a generic client serves both paths.

## B.4 Typical problems (pp.232-235)

| Topic | What goes wrong |
|---|---|
| Client/server sync (B.4.2) | client lacks an accurate clock (browser, generic HTTP stack), drifts, uses another source than the segmenter, unknown ingest delay (MBMS), MPD gives segmenter availability not origin availability, segmenter-to-origin delay unsignalled |
| Segmenter sync loss (B.4.3) | power/cable/CRC loss makes lost media unpredictable and timelines discontinuous; the MPD author may know segments are missing |
| Encoder clock drift (B.4.4) | encoder slaved to TS clock drifts (e.g. 39.97 ms frames vs 40 ms; TS tolerance ~1 frame per 18 min); 23009-1 A.8 handles media-timeline drift but not the effect on availability times; clients fetching by availability time only underrun or add delay; minutes of drift over hours seen in practice |
| Segment unavailability (B.4.5) | a 404 may mean time desync; with MBMS also transport loss that the client middleware cannot signal over HTTP |
| Redundant tool swap (B.4.6) | encoders with different timelines, lost segment numbering after segmenter failure |
| CDN (B.4.7) | cache poisoning by corrupt segments, inconsistency between dual origins with slightly different media times |
| End-to-end latency (B.4.8) | sum of encode delay, upload, edge fetch, player fetch, distance behind live, player buffering; proportional to segment duration without chunked transfer; further behind live = more stable |
| Buffer/bandwidth (B.4.9) | fetching right at availability start maximises buffer but risks starvation |
| Start-up (B.4.10) | audio and video segments are not aligned; client must schedule presentation on the common media timeline |

## B.5 Advanced use cases (p.235)

7 live with undetermined end (MPD updates with ~30 s warning, unknown URLs); 8 24/7 live with canned (static) ads; 9 24x7 with media-time discontinuities (programmes and ads spliced from independent timelines); 10 24x7 with segment discontinuities (encoding interruptions, timelines resume after lost segments).
