---
title: "Clause 10.20 Low latency presentation (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 10.20.1-10.20.8
pdf_pages: 103-113
printed_pages: 103-113
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch10-9-low-latency.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 10.20 Low latency presentation (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Related: ServiceDescription/Latency/PlaybackRate/ProducerReferenceTime/LeapSecondInformation are ISO/IEC 23009-1 (Annex K and 5.x), cited not restated ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)); `prft` box [isobmff-prft](../../../../transmux/docs/README.md#private-spec-transcriptions); MPD-side constraints 4.2.9 and clock sync 4.7.3 in [ch04](ch04-dash-constraints-http-sync.md); provider guidance 11.18 in [ch11](ch11-content-provider-guidelines.md); sample MPD in [annex-d-e](annex-d-e-examples.md).

## 10.20.1 Applicability (p.103)

Low-latency players shall follow the clause. After a seek or MPD anchor away from the live edge the rest of the clause is not required (use 10.9.2). A position within the maximum latency but beyond the target latency lets the player choose either mode. A seek/anchor at the live edge (within target) or in the future requires the clause.

## 10.20.2 Media access and adaptation (p.103)

Players shall parse `SegmentTemplate@availabilityTimeOffset` and `@availabilityTimeComplete`. With `availabilityTimeComplete="false"` and early requests, segments arrive progressively, so whole-request throughput under-measures network capacity. Adaptation shall (a) handle progressive arrival, (b) respect signalled latency limits (10.20.3), (c) still trade quality against reliability. Informative notes: measure over HTTP chunks/frames or CMAF chunks, or probe with cached data (bandwidth-prediction literature; dash.js implements such algorithms).

## 10.20.3 ServiceDescription handling (p.104)

- A `ServiceDescription` may sit in the MPD or a Period (precedence per 23009-1 Table 4). Players treat themselves as in scope when its `Scope@schemeIdUri` is `urn:dvb:dash:lowlatency:scope:2019`; with no Scope element it applies to everyone (23009-1 K.4.1). If several apply, one with this Scope wins; if still ambiguous the behaviour is undefined.
- Worked examples (informative): target 6 s, max 10 s, reference time 2 s at media start, video segments 3.84 s with `availabilityTimeOffset` 2.88 s: latency from chunk availability to target = (6 - 2) - (3.84 - 2.88) = 3.04 s; minus minBufferTime 0.6 s and 2 x 0.02 s decoder/presentation allowance leaves 2.4 s of smoothing buffer; a measured 99.97th-percentile response time of 2.057 s fits, so the target is achievable. With target 5 s the buffer is 1.4 s and the player concludes the target is unreliable.

## 10.20.4 Presentation-time target (p.104-106)

When a `Latency` element applies and has `@target`, the player shall first decide whether the target is likely achievable, considering decoder/pipeline needs, content (segment duration, bandwidths), prior experience and start-up delay limits (10.20.7.4).
Informative derivation of "target latency relative to chunk availability" (Figure 2):
1. PRT0 = (`ProducerReferenceTime@presentationTime` - `SegmentTemplate@presentationTimeOffset`) / `@timescale` + Period start, subtracted from `ProducerReferenceTime@wallClockTime` (format depends on any contained UTCTiming scheme); the reference is the only `ProducerReferenceTime` or the one matching `Latency@referenceId`.
2. TO = `MPD@availabilityStartTime` - PRT0.
3. Latency relative to segment availability = `Latency@target` (ms to s) - TO - segment duration.
4. Add `availabilityTimeOffset` to get the latency relative to chunk availability.
Rules:
- At the live edge, if the target is achievable the player shall present so that a segment start is shown within 500 ms of (`Latency@target` + producer reference time of that segment start); the segment's reference time comes from the presentationTime/wallClockTime mapping of the `ProducerReferenceTime` whose `@id` matches `Latency@referenceId`. Not applicable after pause or seek away from the edge.
- If not achievable, present as close to the target as possible, subject to: for an AdaptationSet marked with Essential/SupplementalProperty `urn:dvb:dash:lowlatency:critical:2019` value `true`, players should not exceed `Latency@max`, and shall not exceed it by more than 500 ms; on reaching the limit they should catch up (10.20.6); if they repeatedly reach it they shall switch to an alternative set without that marker, if any.
- Latency changes inside a Period need not be followed. In-segment `prft` contradicting the MPD mapping may be ignored; the MPD mapping governs.
Figure 2 (p.105), described: timeline with availabilityStartTime, segment duration, availabilityTimeOffset leftward from segment availability to first chunk (C1-C4 per segment), PRT0 and offset TO, and `Latency@target` ending at the target presentation point.

## 10.20.5 Choosing the AdaptationSet (p.106)

A candidate set counts as Low Latency if any of: it has the `critical:2019` marker with value true; `availabilityTimeComplete="false"`; its Representations use `@duration` with all segments 1.5 s or shorter. Low-latency players shall not exclude Role `alternate` sets from candidates. If both low-latency and other candidates exist, choose the low-latency one at start only if (a) the target is judged achievable per 10.20.4 and (b) the player is within any latency constraint, or within 45 s of availability if none.

## 10.20.6 Catch-up (p.107)

- Players shall be able to catch up after buffering. They shall try to catch up when presentation would pass `Latency@max`; where a target exists they should aim for target +/- 500 ms if reliable; they shall not present more than 500 ms earlier than `Latency@min`.
- Seeking forward is a required method, used only when `Latency@max` exists and has been reached (Figure 3).
- Faster-than-real-time playback is optional; if used, no audio pitch change (no pitch correction/silence removal); usable when latency exceeds the target and lower latency looks reliable; never faster than `PlaybackRate@max` when a `PlaybackRate` element applies.
- Content providers choose the target as the best experience; the target may exceed the technical minimum.

## 10.20.7 Joining (p.107-112)

- Informative: random access points are segment starts; joining mid-segment either decodes (faster than real time) from the last start and adds latency, or waits for the next start and adds delay; Figure 4 shows start-up delay and initial latency rising/falling per join time, Figure 5 the trade-offs (switch earlier to "wait for next segment", decode faster, or just present from segment start). Players may trade within the 10.20.7.4 limits and latency constraints.
- Balanced case: AL_max ≈ T_max ≈ D_s / (S + 1) with D_s segment duration and S decode speed multiple (chunking adds some deviation).
- Decoding speed depends on headroom vs the decoder profile/level (signalled by `@maxPlayoutRate`) and the share of non-reference frames. Table 33 minimum decode speeds a low-latency player must reach: `@maxPlayoutRate` >= 2.0 any share of non-reference frames -> 2x; >= 3.0 with more than 40 % -> 3x; >= 4.0 with more than 40 % -> 3.3x (met either by true decode at that rate or by discarding non-reference frames and decoding the rest at up to 2x).
- Start-up delay limits apply when: a 704x396-or-larger video Representation decodable at N x exists; network delivers at least N x the combined bit rate of that video, a stereo-or-better audio Representation and any subtitle Representation; audio chunk duration <= video chunk D_c; subtitle chunks <= D_c or offset so subtitles are available with each video chunk; no external reason to delay; the latency from chunk availability to target presentation, and the gap between target and maximum latency, are each at least the table's maximum start-up time. Table 34: N=1x: D_s/2 + D_c; 2x: D_s/3 + D_c/2; 3x: D_s/4 + D_c/3; 3.3x: D_s/4.3 + D_c/3.3. Informative example: D_s 3.84 s, D_c 0.96 s, 704x396 50 Hz AVC with 50 % non-reference frames, 1600 kbps, `@maxPlayoutRate` 4.5, audio 128 kbps, 6 Mbps network -> maximum 1.18 s.
- Initial latency may vary with join time within any min/max limits; variable-speed playback may then pull it toward the target.

## 10.20.8 Drift and leap seconds (p.112-113)

Sub-second clock sync (4.7.3) is needed to avoid drift eroding the buffer. Low-latency players shall compute availability times using `LeapSecondInformation` and their reference clock's leap-second behaviour: apply `@availabilityStartLeapOffset`, except use `@nextAvailabilityStartLeapOffset` when `@nextLeapChangeTime` and `@nextAvailabilityStartLeapOffset` are both present, `@nextLeapChangeTime` has passed on the reference clock, and that clock has been adjusted for the leap second. A clock driven by UTCTiming may only update at the next poll. Players shall update their reference clock for a signalled leap second within 24 h of it. If next offset > current offset a second is inserted, otherwise removed.

## Notes on the source

- EXAMPLE 2 in 10.20.3 computes 5 000 + 2 057 - 1 400 ms (= 5 657 ms) yet says the player targets 5,08 s; the other examples recompute correctly (p.104, p.112).
- Examples use "TargetLatency"/"MaximumLatency" for what the rules call `Latency@target`/`@max`; `Latency@ReferenceId` vs `@referenceId` for the same attribute (p.104-105).
- Typos in 10.20.1, 10.20.2, 10.20.3, 10.20.4 NOTE 1/6, 10.20.7.4 ("the the", "playerusing", "AdapationSets", "An DASH").
