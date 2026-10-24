---
title: "Clause 10.9 Live services (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.9.1-10.9.7
pdf_pages: 91-94
printed_pages: 91-94
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-5-live-services.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 10.9 Live services (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Cross-references: segment availability window 23009-1 5.3.9.5.3; MPD Anchors 23009-1 Annex C.4; `@minimumUpdatePeriod` ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)); DASH-IF IOP client operation in [dashif-iop-4.3](../dashif-iop-4.3/); clock sync (4.7.3) and MPD update constraints (4.8) in [ch04](ch04-dash-constraints-http-sync.md). Low latency is clause 10.20 ([ch10-9](ch10-9-low-latency.md)).

## 10.9.2 Where to start playing (p.91-92)

| Situation | Start position (player shall) |
|---|---|
| MPD Anchor present in the MPD URL | the anchor's position |
| no anchor, `@type="static"` | start of the MPD |
| no anchor, dynamic, no `@suggestedPresentationDelay` | such that the media shown is no more than 45 s behind its availability time |
| no anchor, dynamic, with `@suggestedPresentationDelay` | no more than the larger of that delay and 45 s behind |
45 s = three times the maximum audio/video segment duration set in clause 4.5 (the standard gives this as the rationale). Anchor examples (fragment syntax): `#t=0`; `#period=<Period@id>&t=50:00` (the source prints spaces around `=`). Without `period`, `t` follows W3C Media Fragments 4.2.1; the Period id is URL-coded and must match. An anchor should name an available time; an unavailable one follows Media Fragments error handling. A player shall never request outside the availability window.

## 10.9.3 Template-addressed segments (p.92)

With `$Number$` and constant duration a player can compute names up to the next MPD update, but shall check availability before requesting and shall not request unavailable segments (early 404s are harmful to caching). When playing behind live and MPD updates come through inband events (9.1.4), request and process a new MPD before asking for a segment number above any requested so far; after a forward seek without refresh, be ready to fetch a new MPD if the segment is unavailable (10.8.6).

## 10.9.4 24/7 operation (p.93)

- `availabilityStartTime` may be far in the past: players shall support at least one year back. MPD time ignores leap seconds.
- Segment numbers: support at least 10 decimal digits (with leading zeros possible).
- Track fragment decode times: support 64-bit values.
- `SegmentTimeline`: if the last `S` entry's `d` differs from the decoded segment's actual duration, add the actual duration to the previous `t` to get the next `t`; this extends the timeline without an MPD update, whatever the value of `@availabilityTimeComplete`.

## 10.9.5 MPD update timing (p.93)

With a dynamic MPD that has `@minimumUpdatePeriod` and no inband expiration events, players should refresh at the minimum rate needed (23009-1 A.2) and shall never poll more often than `minimumUpdatePeriod - 2 x (longest segment duration in the MPD)`.

## 10.9.6 Time-shift buffer (p.93-94)

Players shall parse `@timeShiftBufferDepth` (absent = infinite) and use it for the available range from the live edge; it may not be updated as segments accrue, so long-ago start times are a risk. Before each segment request a player shall check both: (1) now lies between the segment's availability start and end (23009-1 5.3.9.5.3); (2) the segment exists in the MPD: for `SegmentTimeline`, it is listed in an `S`; for `@duration` + `$Number$`, its earliest presentation time is not before the Period start (or earliest known segment time) and its number is not below `@startNumber`. If both hold the request is safe.

## 10.9.7 Dynamic MPD turned static (p.94)

Detection after an MPD update: `@mediaPresentationDuration` added, `@minimumUpdatePeriod` removed, `@type` changed to static (`@availabilityStartTime` possibly removed). Player shall then stop updating the MPD and continue from the current media presentation time (the segment's time from `t` or computed from `@duration`). A converted MPD may set Period start 0 with `SegmentTemplate@presentationTimeOffset` at the earliest segment time; a first-time requester plays it from the beginning. Provider side: clause 11.19.

## DVB additions / disagreements

- The 45 s start cap, the poll-rate floor (10.9.5), the 1-year / 10-digit / 64-bit support minimums and the 10.9.6 two-part availability check are DVB statements; DASH-IF IOP chapter 4 has its own live-edge/refresh rules (see the IOP notes). A client targeting both profiles must satisfy both; where numbers differ, DVB's are the stricter player rules for DVB profile content.
- Source typo/oddity: 10.9.4.1 says `availabilityStartTime` uses the "XML Period type".
