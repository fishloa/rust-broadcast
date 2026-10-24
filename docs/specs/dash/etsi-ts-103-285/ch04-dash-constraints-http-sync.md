---
source: private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
document: ETSI TS 103 285 V1.4.1 (2023-09)
clauses: 4 DASH constraints, HTTP support and synchronization (4.1-4.8.4)
pdf_pages: 19-27 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch04-dash-constraints-http-sync.md
verified_against_render: true
notes_original_wording: true
---

# TS 103 285 - clause 4 (notes): MPD, segment, timing and MPD-update rules

Original-wording notes; strengths (shall/should/may) are the spec's. "Ignorable" below means the spec says a player
"may"/"can" skip that construct. Base rules: ISO/IEC 23009-1 (see `transmux/docs/codec/dash-23009-1-2022-*.md`);
DASH-IF counterpart of 4.7/4.8: `../dashif-iop-4.3/`.

## 4.1 Profile (pp. 19-20)
The DVB profile merges the ISO BMFF live and on-demand profiles, allows xlink only with `actuate="onLoad"`, and
excludes multiplexed representations, SegmentList addressing and xlink `onRequest`. Two profile URNs:

| URN | Use |
|---|---|
| `urn:dvb:dash:profile:dvb-dash:2014` | each media type present has at least one AdaptationSet usable by a player without HDR, HFR or NGA support |
| `urn:dvb:dash:profile:dvb-dash:2017` | at least one video/audio AdaptationSet needs HDR (5.2.6/5.2.7), HFR (5.2.8), NGA (6.3.2, 6.8) or correct EssentialProperty handling |

## 4.2 MPD constraints (pp. 20-22)
- General: MPD shall follow 4.2 on top of 23009-1 clause 7.3; non-conforming parts may be ignored; a DOCTYPE
  declaration shall not appear.
- Common Period rules: Subset may be ignored; Period.SegmentList shall not appear; with several `contentType="video"`
  AdaptationSets one shall carry Role `urn:mpeg:dash:role:2011` = `main`; remote elements with `actuate="onRequest"`
  may be ignored; a remote Period shall not carry `xlink:actuate` (avoids loops); each Period is either live-type
  (4.2.3-4.2.5) or on-demand-type (4.2.6-4.2.8), never a mix.
- Live Periods, AdaptationSet (4.2.4) - ignorable if: no `@contentType`; absolute BaseURL; has ContentComponent
  (multiplexed); no SegmentTemplate on the set or on every Representation (SegmentTimeline is allowed); more than one
  Representation unless `@segmentAlignment` is true/1, `@startWithSAP` is 1 or 2, and `MPD@maxSegmentDuration` is
  present or `MPD@type="static"`. `@group` may be ignored. Segment alignment across different AdaptationSets is not
  required. Pairs of Representations that 10.4 does not require switching between may legitimately share a set.
- Live Representation (4.2.5) - ignorable if: `@mimeType` is not `video|audio|application|text` + `/mp4`
  (extra parameters may be appended); inferred `@profiles` is not `urn:dvb:dash:profile:dvb-dash:isoff-ext-live:2014`
  (no 2017 variant exists); absolute BaseURL.
- On-demand Period (4.2.6-4.2.8): Period.SegmentTemplate shall not appear. AdaptationSet ignorable if: no
  `@contentType`, ContentComponent present, not every Representation has BaseURL, SegmentList/SegmentTemplate present,
  or (several Representations) `@subsegmentAlignment` is not true or `@subsegmentStartsWithSAP` is not 1/2.
  Representation ignorable if: `@mimeType` rule as above; `@profiles` is not
  `urn:dvb:dash:profile:dvb-dash:isoff-ext-on-demand:2014`; has SegmentList/SegmentTemplate; no BaseURL; or a single
  indexed (self-initialising) segment without `SegmentBase@indexRange`.
- Low latency (4.2.9): chunked segments offered before completion use `SegmentTemplate@availabilityTimeOffset` plus
  `@availabilityTimeComplete="false"` (the BaseURL variants are not used for this). The offset shall not exceed
  `@duration / @timescale`. Worked example in the spec: 3.84 s segments, 0.96 s chunks -> offset 2.88 s. Short segments
  that need no progressive delivery need neither attribute. A ServiceDescription with at most one Latency and one
  PlaybackRate should describe the target, at MPD or Period level; Scope scheme
  `urn:dvb:dash:lowlatency:scope:2019` addresses players of this document.

## 4.3 Segment format (pp. 22-23)
For both live-type and on-demand-type Periods: formats per 23009-1 clause 7.3; `sidx`/`ssix` boxes, if any, precede all
`moof`; subtitle segments available no later than the media they accompany; one sample-entry type per AdaptationSet
(no `avc1`/`avc3` or `avc3`/`hev1` mix); exactly one `traf` per `moof`; one `track_ID` across the Representations of a
set; CMAF track/header/fragment conformance is "should", signalled by `@segmentProfiles` brands (23000-19 7.2).
The spec also reminds (citing 23009-1 6.3.4.2) that a media segment may hold several `moof`/`mdat` pairs, from one per frame/audio sample up to a single pair. On-demand-type Periods additionally: one segment per
Representation, self-initialising, a single `sidx`; CMAF track-file conformance should (23000-19 7.3.3.3).

## 4.4 Attribute presence (pp. 23-24)
- Video AdaptationSet should carry `@maxWidth`/`@maxHeight`/`@maxFrameRate` (or the plain value if uniform) and `@par`;
  the max values describe target display size and may exceed the largest coded Representation.
- Each video Representation shall have (own or inherited) `@width`, `@height`, `@frameRate`, and `@scanType` when any
  Representation in the set is interlaced. `@sar` should be present or inherited.
- If the picture aspect is not 16:9, `@par` or `@sar` shall be present or inherited.
- A Representation/AdaptationSet containing FailoverContent may be ignored.

## 4.5 Dimension limits (p. 24, all mandatory)

| Item | Limit |
|---|---|
| MPD size (before and after xlink resolution) | <= 256 Kbytes |
| Periods | <= 64 (before and after xlink) |
| AdaptationSets per Period | <= 16 |
| Representations per AdaptationSet | <= 16 |
| Segment duration | >= 960 ms (last segment of a Period may be shorter); chosen for audio/video chunk alignment in 50 Hz systems |
| Subtitle segment size | <= 512 Kbytes |
| Video / audio segment (no subsegments) and each subsegment | <= 15 s |

## 4.6 Servers (p. 24)
HTTP/1.1 (RFC 7230) and cookies (RFC 6265) shall be supported.

## 4.7 Availability-time synchronisation (pp. 24-26)
Applies when the MPD is dynamic or has `availabilityStartTime`.
- Provider (4.7.2): the MPD should carry a UTCTiming element with one of `urn:mpeg:dash:utc:` + `ntp:2014`,
  `http-head:2014`, `http-xsdate:2014`, `http-iso:2014`, `http-ntp:2014` (the spec's list omits `direct`). Without
  UTCTiming, segments shall be available by the announced time against a globally accurate source within 200 ms. With
  UTCTiming, the timing source shall be reachable and segments shall be available by the announced time for every
  device using those methods. With `http-head`, the server must return a Date header reflecting now; cached (CDN)
  responses distort it. Leap seconds can affect long-running and low-latency streams (signalling: clause 11.18.2.4).
  Servers may use a different time zone than the MPD/players.
- Player (4.7.3): without UTCTiming, use an accurate system wall clock and allow for its inaccuracy near availability
  boundaries (should). With `http-head` or `http-xsdate`: shall sync to one of the listed sources and shall not request
  a segment before its availability start time per the chosen method (may allow for accuracy/transit delay). If a
  segment is not available at its start time, shall use the recovery methods of clause 10.8. Should not poll the time
  server more than needed (not per segment). Shall handle at least the integer seconds of fractional `http-xsdate`
  (and `http-iso` where supported); low-latency players shall resolve sub-second time.
- Page-26 note: the text-layer conversion had mangled the http-iso sentence; the render was used.

## 4.8 MPD updates, `@type="dynamic"` (pp. 26-27)
- Players shall support: adding Representations/AdaptationSets to a Period (never removing); adding/removing/updating
  `S` entries in SegmentTimeline; dynamic -> static (see 10.9.7, 11.19); BaseURL changes in a Period;
  adding/removing events in EventStream; adding/removing InbandEventStream/EventStream; adding or removing a Period
  as long as references to available segments remain.
- Optional for players: change of `Representation@bandwidth`; change of the UTCTiming element.
- Constraints on the server (shall not change for a corresponding element): `timeShiftBufferDepth`,
  `availabilityStartTime`, `maxSegmentDuration` (not larger), Period `AssetIdentifier`, Period `@id` and `@start`, Role,
  AudioConfiguration, `SegmentTemplate@timescale`, `@contentType`, `@codecs`, `@id`, `@lang`.
- Required: `MPD@publishTime` set to the publish time.
- Note: the spec writes "Period@AssetIdentifier" as if an attribute; in 23009-1 it is a child element.

See also: `specs/rules/emsg-rules.md` and `mp4-emsg/docs/emsg.md` (inband events behind 4.8.1 items 5-6).
Clauses 10.4 (seamless switching), 10.8 (recovery), 11.18/11.19 are in sibling files.
