---
title: "Clause 11 Content provider guidelines (notes; informative clauses relevant to players)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 11.1, 11.4-11.19
pdf_pages: 113, 118-129
printed_pages: 113, 118-129
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch11-content-provider-guidelines.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 11 Content provider guidelines (notes; informative clauses relevant to players)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Not covered here (not client-relevant): 11.2-11.3 (recommended resolutions/frame rates and audio bit-rate tables, Tables 35-40; pp.113-117). Normative player behaviour these notes explain: [ch10-4](ch10-4-resiliency.md) (BaseURL), [ch10-5](ch10-5-live-services.md), [ch10-9](ch10-9-low-latency.md).

## 11.1 Profile URNs (p.113)

Representations meant for DVB players should infer an `@profiles` that includes a clause 4.1 name plus the Live (4.2.5) or On Demand (4.2.8) one. If correct presentation of a video/audio set depends on an EssentialProperty, do not label it with the 2014 URN; use the 2017 URN. The `urn:dvb:dash:fontdownload:2014` EssentialProperty may be used in subtitle sets under either URN.

## 11.4-11.7 Adaptation Set composition and subtitles (p.118-119)

- Interlaced and progressive may share a set; strongly recommended only for 1080i mixed with progressive.
- Mixing HD and SD: preferably derive an SD ladder from the HD master (BT.709) in one set (one licence). If licences must differ, use two sets; HD may still adapt down into the SD set at CDN cost.
- Different video codecs in one AdaptationSet are not allowed (4.3); use one set per codec.
- Subtitles: EBU-TT-D documents in ISO BMFF. Subtitle segments may be longer than A/V ones (limit: max subtitle segment size, 4.5, roughly 3 hours) but for live they must be available no later than the matching media (equal durations, or earlier availability through `availabilityTimeOffset`). Each sample should contain only subtitles shown during it (one sample per segment is expected; players clip to sample start/duration). Subtitle Representations should have no discontinuities (empty documents per EBU Tech 3381 clause 6 fill gaps and pad the start/end to match A/V duration). gzip helps. Content should stay in the intersection of clause 7 and the CMAF `im1t` (IMSC1 Text) profile.

## 11.8 Period boundaries (p.119)

Seamless switching across Periods needs matching AdaptationSet properties on both sides.

## 11.9 BaseURL signalling guidance (p.119-122)

- Intent: coarse load balancing and failover between CDNs/data centres, not between many individual web servers.
- Priority: lower value first; only the order matters (1,2 equals 1,4); equal priorities form a load-balancing group; e.g. cheap/moderate/costly CDNs at priority 1/2/3.
- Weights matter only within a priority; chance = weight / sum. Examples: equal 1/1 for two CDNs; 9/9/2 puts 10 % on the third.
- Combining: a failure blacklists the `serviceLocation` and, by design, every BaseURL sharing a priority with it, so a small CDN is not overloaded after a peer fails. The worked set has priorities 1 (cdn1 9, cdn2 9, cdn3 2), 2 (cdn1, cdn2 1:1: after cdn3 fails), 3 (cdn1 9, cdn3 1: after cdn2), 4 (cdn2 9, cdn3 1: after cdn1), 5 (cdn1) , 6 (cdn2), 7 (costly-cdn). Figure 6 (p.121) charts every route; as printed it shows a 45 / 10 / 45 percent first-level split for cdn1/cdn2/cdn3, which does not match the 9/9/2 weights (45/45/10 would).
- Hygiene: put absolute BaseURLs at one level; do not mix absolute and relative at a level; do not include BaseURLs that will be ignored; no several relative BaseURLs in one element; relative ones carry no `serviceLocation`/`priority`/`weight`. Typical layout: absolute set at MPD (or Period) level, single relative BaseURLs below.

## 11.10-11.13 Events, assets, 24/7, caching (p.122-124)

- Avoid the MPD Patch event. Where v1 `emsg` is needed for compatibility add duplicate v0 events for v0-only players.
- Make events available at least 4 s before their time: MPD events via an MPD with `@publishTime` at least 4 s + the minimum update period earlier; inband events via a segment whose earliest time is at least 4 s earlier.
- AssetIdentifier: unique per asset within the MPD (content and ads), same `@value` for all parts of an asset; Periods with the same identifier should have the same Representations and init segments (lets players share init data); offer period-continuous sets if identifiers repeat.
- 24/7: A/V alignment cannot rely on both starting at 0; redundant paths need identical segment names, track fragment decode times and fragment sequence numbers (timecode-locked segmenting; careful audio/video offset when swapping audio encoders). Key rotation needs thought (volume per key, IV reuse, entitlements) and keys/licences must reach players well ahead, especially for low latency. See IOP clause 4.
- Cache control: dynamic MPDs need caching headers that let players fetch updates when `@minimumUpdatePeriod` or expiration events demand.

## 11.14-11.17 HDR, HFR, subtitles, NGA terms (p.124-127)

- Non-BT.709-compatible colour: use EssentialProperty per 5.2.5; HDR rules in 5.2.6/5.2.7. Players are not required to follow colour-space or transfer changes within a Period; convert to one format or use several Periods.
- High frame rate = above 60 Hz (100/120 Hz). A Representation must be self-contained; it includes exactly the temporal layers it needs. Examples (pp.124-126): five resolutions with 50 Hz and 100 Hz variants where sub-layer 0 (or 0+1) is carried and `urn:dvb:dash:highest_temporal_id:2017` SupplementalProperty values 0/1 (two layers) or 0/1/2 (three layers) mark the highest layer; HEVC codec strings `hev1.2.4.L120/123/150/153/156.00.00.B0`, timescale 1000, segment duration 3840.
- Subtitles must be decodable by an EBU-TT-D renderer.
- NGA: component-based audio (dialogue, ambience, commentary...), also "full-main" pre-mixes. An audio bundle = components that can feed one NGA decoder; a Preselection = a user-selectable subset for simultaneous decoding (an NGA analogue of an audio service), only from one bundle, mapped to DASH Preselections. MRMP content that spans AdaptationSets shall offer the same number of Representations in each set (11.17.3).

## 11.18 Low latency MPDs (p.127-129)

- Two techniques: short segments (compatible, but many requests; avoid SegmentTimeline so the MPD need not reload; keep segments at least 1 s for older players) or multi-chunk segments (CMAF 23000-19 6.6.5, delivered with chunked transfer encoding; some older players may fail).
- If both low-latency and legacy players read the MPD, mark low-latency sets with `urn:dvb:dash:lowlatency:critical:2019` value true: Essential hides them from non-supporting players, Supplemental lets them through.
- MPD should not need frequent reloads; video chunks need not be shorter than the maximum reorder distance (usually the P-frame distance); audio chunk duration should match video.
- Time: include `UTCTiming` with sub-second accuracy and `LeapSecondInformation`; announce an upcoming leap second by `@nextLeapChangeTime`/`@nextAvailabilityStartLeapOffset` early enough, and not within 24 h after a previous one. With a `ProducerReferenceTime` also add UTCTiming inside it (23009-1 5.12.2 then requires the same one in the MPD; its `wallClockTime` format follows the scheme, default NTP timestamp); set `ProducerReferenceTime@inband` false or keep inband data consistent with the MPD.
- Availability: either align MPD availability with the worst case (costing latency) or make servers accept requests at the stated time even if first bytes come late.
- Inband events: at segment start as usual; or at chunk starts repeated at the next segment start (mid-segment processing is not required of players); or shorten audio segments instead of chunking.
- Keep the same latency requirements on both sides of a Period boundary for seamless low-latency continuation.

## 11.19 Dynamic to static (p.129)

When a live stream ends, providers may turn the MPD static: add `@mediaPresentationDuration`, remove `@minimumUpdatePeriod`, set type static, optionally drop `@availabilityStartTime` and `@timeShiftBufferDepth`, set Period start 0 and `@presentationTimeOffset` to the earliest media time per AdaptationSet. Expectation: current players continue, new players start at the beginning. Player behaviour: 10.9.7.

## Notes on the source

- Typos: "characterstics", "playerss", "adaption", "targetted", "cd2"; comment markers printed with an em dash in the BaseURL examples.
- 11.19 lists seven provider-side changes, 10.9.7 gives three player-side detection conditions; they describe the same event from two sides.
