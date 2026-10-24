---
source: private/specs/dashif_iop_v4.3_guidelines.pdf
clauses: 3.10 On-Demand Services (3.10.1-3.10.4, Tables 5-6)
pdf_pages: 73-82
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch03-10-on-demand-services.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# DASH-IF IOP v4.3 — 3.10 On-demand services (notes)

PDF p.N = printed p.N-21. Verbatim: private file above. Chapter 4 begins at the bottom of PDF p.82 (not here).

## 3.10.1 Introduction (p.73)

On-demand = `MPD@type` static: all media is available from `availabilityStartTime` until `availabilityEndTime`, so the client may consume at any time, pause and use trick modes. Static services can use live profiles (3.10.2) or on-demand profiles (3.10.3); live profiles suit content that was live and is now offered as catch-up/time-shift. The clause defines an IOP for on-demand based on the 2018 third edition of 23009-1.

> **Notes:** the first bullet repeats a sentence tail; the last sentence ends "in clause." (number missing); "clause 4.6.3.10.2" has no match in the table of contents (4.6.3 is "Client Behavior").

## 3.10.2 On-demand with live profiles (pp.73-77)

Table 5 (MPD information) — name: status:
- `MPD@type`: optional, static if present; `@availabilityStartTime`: optional (AST; absent = all available); `@mediaPresentationDuration`: mandatory unless Period duration present; `@minBufferTime`: mandatory (MBT, see 3.2.8); `Period@start`: mandatory for the first Period; `SegmentTemplate@media`: mandatory; `@startNumber` and `@timescale`: "optional default"; `@duration` or `SegmentTimeline`: exactly one per Representation.

Static MPD rules (3.10.2.2.2): `@type` static; `@mediaPresentationDuration` present or the last Period's `@duration` present; `@minimumUpdatePeriod` shall not be present; `@timeShiftBufferDepth`, `@suggestedPresentationDelay`, `@availabilityTimeOffset` should be absent and clients ignore them if present.

Derivation (a client derives the segment list at time NOW):
- Period start `PSwc[i]`: `@start` minus the first Period's `@start`; if no `@start`, previous start + previous `@duration` (`@start` wins if both). Period end `PEwc[i]`: for the last Period, MPDur = `@mediaPresentationDuration` or PSwc(last) + `@duration`(last); otherwise `PSwc[i+1]`.
- Segment repeat counts, Ns S-elements indexed s=1..Ns, ts = `@timescale`: with `SegmentTimeline`, t[s], d[s] from `@t`, `@d`; r[s] = `@r`+1 if `@r` >= 0; else (negative) ceil((t[s+1]-t[s])/d[s]) when a next S exists, otherwise ceil((PEwc-PSwc-t[s]/ts)*ts/d[s]) (parentheses unbalanced as printed). With `@duration` only: Ns=1, t=0, d=`@duration`, r = ceil((PEwc-PSwc-t/ts)*ts/d), minus one if the last segment does not exist. A `@r` smaller than the timeline end implies a gap with no media.
- Parameters per Period/Representation: PTO o, first/last segment numbers k1/k2, MPD start/end of init segment MST[0]/MET[0], per segment MST/MET/SD, URL. Pseudo-code: k=0, MST[0]=PSwc; for each S: k+=1, MST[k]=PSwc+(t+d-o)/ts (as printed, including +d), SD=d/ts, MET=MST+d/ts, URL by `$Number$` (start at `@startNumber`) or `$Time$` (address t); then r repeats: k+=1, MST[k]=MST[k-1]+d/ts, MET and SD likewise, address +1 or +d; finally k2=k and MET[0]=MET[k2]. Uses `URLTemplate(ReplacementString, Address)` of 4.3.2.2.8.
- Media time of a segment: see 3.2.7/3.2.11. Synchronized playout and seamless switching use the earliest presentation time inside the Period (cites "5.2.2.5").

> **Notes:** "Table X 5" is a field-reference remnant (the 5 is highlighted); the first-segment `MST` formula includes `+ d[s,i,r]` as printed; prose says "MSD", pseudo-code "SD".

## 3.10.3 On-demand with the On-Demand profile (pp.77-82)

- Motivation: one file indexed by `sidx` avoids millions of files; benefits for providers, caching, encoding (flexible IDR, capped VBR) and clients (flexible request sizes, trick modes). A simple live client only adds: fetch and parse the Segment Index to get byte ranges, then request those ranges.
- Profile `http://dashif.org/guidelines/dash-if-ondemand`: extended ISO BMFF On-Demand profile (23009-1 8.9); one Indexed Self-Initializing Segment per Representation (6.3.5.2); `@indexRange` present; one `sidx`. Notes repeat 3.2.1's three notes. It points to "clause 5.3.2/5.3.3" for further guidance, which in this document are the ad-insertion mapping/workflow clauses (mis-reference, p.79).
- Table 6 (status): `MPD@type` optional static; `@availabilityStartTime` optional; `@mediaPresentationDuration` mandatory unless Period duration; `@minBufferTime` mandatory; `Period@start` optional, default 0 (first Period is an anchor); `AdaptationSet@subsegmentAlignment` mandatory true; `@subsegmentStartsWithSAP` mandatory 1 or 2; `Representation@bandwidth` (see above); `Representation.BaseURL`; `SegmentTemplate@timescale` optional default 1; `@presentationTimeOffset` optional default 0; `SegmentTemplate@indexRange` mandatory, one RFC 7233 byte-range-spec (contiguous). All Table 6 parameters shall be present.
- Single `sidx` settings: `reference_ID` = track_ID; `timescale` = media header timescale; `earliest_presentation_time` typically 0 (more if split from a larger file or for A/V sync); `first_offset` points to the first movie fragment header; `reference_count` = number of subsegments/fragments; `reference_type` all 0; `referenced_size` = bytes to the next fragment (to end of file for the last); `subsegment_duration` = difference of earliest presentation times of consecutive fragments; `starts_with_SAP` = 1, `SAP_type` 1 or 2; `SAP_delta_time` 0. Box fields: reference_ID, timescale, earliest_presentation_time and first_offset (32-bit for version 0, 64-bit otherwise), reserved, reference_count, then per reference reference_type(1), referenced_size(31), subsegment_duration(32), starts_with_SAP(1), SAP_type(3), SAP_delta_time(28).
- Client: extend the `moov` request to cover the `sidx`, build (URL, byte range) pairs; a smart client may fetch only the start of the `sidx` first; request size may be independent of segment duration; fully parsing every Representation's `sidx` gives the subsegment map. Multiple Periods: as 3.2.12, may be continuous and connected.

## 3.10.4 Mixed on-demand (p.82)

Profile `http://dashif.org/guidelines/dash-if-mixed-ondemand`: multiple Periods; every Period's Representations conform to either a live profile (3.10.2) or the on-demand profile (3.10.3); `@profile`(sic) signalling in each Period. A client shall support multi-Period offerings and mixed on-demand/live periods. Dynamic service types are not supported.

> **Note (disagreement):** Table 1 gives the identifier as `.../dash-if-mixed`, this clause as `.../dash-if-mixed-ondemand`.
