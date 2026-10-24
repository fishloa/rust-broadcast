---
title: "Annexes D and E (informative) Reporting examples and low-latency MPD (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: Annex D (D.1, D.2), Annex E
pdf_pages: 146-148
printed_pages: 146-148
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/annex-d-e-examples.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Annexes D and E (informative) Reporting examples and low-latency MPD (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Normative text: [ch10-7](ch10-7-reporting.md) (reporting) and [ch10-9](ch10-9-low-latency.md) (low latency). Full URLs/XML: private verbatim file.

## D.1 DVBErrors (p.146)

An MPD with two BaseURLs (priority 1 / serviceLocation `CDN2`, priority 2 / `CDN3`) and `Metrics metrics="DVBErrors"` whose `Reporting` has scheme `urn:dvb:dash:reporting:2014`, value 1, `dvb:reportingUrl` an example `dash_error` endpoint, `dvb:probability="500"`. A player that draws "reporting" (chance 0.5) first sends one GET with `errorcode=S00` (empty `url`/`ipaddress`, `servicelocation=CDN2`). On an HTTP 503 for a media segment followed by a BaseURL switch it sends two GETs: `errorcode=503` (with the segment URL, resolved IP, `servicelocation=CDN2`) and `errorcode=F00` (empty `url`/`ipaddress`). Query keys in order: `mpdurl` (URL-encoded), `errorcode`, `terror`, `url`, `ipaddress`, `servicelocation`.

## D.2 Low-latency metrics (pp.146-147)

MPD fragment: `availabilityStartTime` 1970-01-01, one BaseURL (`CDN1`), a video set (`@duration` 3840 at timescale 1000, Representations `1280x720@50` and `960x540@50`) and `Metrics metrics="HttpList(50,MediaSegment),RepSwitchList,PlayList"` with the DVB reporting descriptor at `probability` 1000.
Scenarios (all as GETs with `metricname`):
- RepSwitchList: keys `t` (wall clock), `mt` (media time ms), `to` (Representation id, `@` as `%40`); two switches 20 s apart.
- HttpList: keys `type=MediaSegment`, `url`, `trequest`, `tresponse`, `responsecode`, `interval=50`, `trace` (URL-encoded `s`, `d`, `b` list of per-interval byte counts); two successive segment downloads (first byte after 61 ms and 48 ms; bodies over 1352 ms and 756 ms).
- PlayList: keys `start`, `mstart`, `starttype` (`NewPlayout`, `Resume`), `trace` (objects with `representationid`, `start`, `mstart`, `duration`, `playbackspeed`, `stopreason` such as `Rebuffering`, `UserRequest`, `EndOfPeriod`); a session with 26.3 s play, 4.13 s rebuffer, a pause of 42.672 s and play to Period end yields two reports (at pause, at Period end).

## Annex E Low-latency MPD example (p.148)

Dynamic MPD, profiles `urn:dvb:dash:profile:dvb-dash:2014` and `...:isoff-ext-live:2014`, `minBufferTime` PT1.143S, `maxSegmentDuration` PT3.84S, `minimumUpdatePeriod` PT1H, `timeShiftBufferDepth` PT2M, `availabilityStartTime` 2019-08-06T13:31:00Z, `publishTime` 2021-06-03T13:00:00Z. `ServiceDescription id=0` with `Scope` `urn:dvb:dash:lowlatency:scope:2019` (value 2), `Latency min=4800 max=34800 target=6800 referenceId=7`, `PlaybackRate min=0.96 max=1.04`. One Period (start PT0S): video set (`startWithSAP=2`, `par=16:9`, inband `urn:mpeg:dash:event:2012` value 1, `ProducerReferenceTime id=7` with wallClockTime 2019-08-06T13:44:12Z and presentationTime 158400 containing a `UTCTiming` http-xsdate, Role main, `SegmentTemplate` startNumber 404547501, presentationTimeOffset 310692480000, timescale 200, duration 768, `availabilityTimeOffset` 2.88, `availabilityTimeComplete=false`, one `avc3.640020` 1280x720 50 fps 5447392 bps progressive Representation) and an audio set (`mp4a.40.2`, 48000 Hz, `eng`, 5.1 channel config value 6, same inband event stream, startNumber 404547501, offset 74566195200000, timescale 48000, duration 184320, same availability settings, 319520 bps). MPD-level `UTCTiming` (http-xsdate) and `LeapSecondInformation availabilityStartLeapOffset=37`.

## Notes on the source (examples not corrected)

- D.2: the Representation ids use `@` (`1280x720@50`), the PlayList trace and Annex E use `p` (`1280x720p50`).
- D.2 URLs contain stray spaces and the first HttpList trace is cut mid-number; "756 ms.The" missing space; the RepSwitchList media times differ by 19.2 s while the wall clock times differ by 20 s.
- Annex E's profiles value is split across a line after `dvb-` in print.
