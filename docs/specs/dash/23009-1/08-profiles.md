---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 8 - Profiles (8.1 to 8.12)"
pdf_pages: "186-217"
printed_pages: "178-209"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/08-profiles.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Clause 8 - Profiles (implementer notes)

These notes restate, in our own words, what clause 8 of ISO/IEC 23009-1:2022 requires. The tool-generated verbatim conversion of the
licensed PDF lives in the private submodule at `private/specs-verified/dash/23009-1/08-profiles.md`. Each section below cites the
clause and printed page. Profile identifiers are quoted exactly, as they are facts. Clause numbers 5.x, 6.x and 7.x referenced below
are covered by [PR #1516's files](INDEX-C1.md#sibling-coverage) or are not yet transcribed.

## 8.1 What a profile is (pp. 178-180)

- A profile is a bundle of restrictions. It usually limits MPD features and Segment formats, but a profile defined elsewhere may also
  limit codecs, bit-rates, picture sizes and so on. Profiles defined in this document restrict only MPD features and Segment formats,
  never codec types.
- The standard does not define client behaviour, so a profile is a restriction on *content* (MPD plus Segments). It also serves as
  permission for a client that implements only the profile's features to process the presentation (Note 1, p. 179).
- A profile identifier is a URI and must not contain a comma, because `MPD@profiles` is a comma-separated list. Identifiers defined
  here are URNs (RFC 8141). External profiles may be URNs or URLs; a URL should embed a month-year (`mmyyyy`) for the date the
  domain owner authorised it.
- External parties are encouraged to call their own definitions "Interoperability Points" rather than profiles.

### Conformance procedure (p. 179-180)

To check conformance to a profile `ProfA` listed in `MPD@profiles`, derive a *profile-specific MPD*:

1. Set `MPD@profiles` to just `ProfA`.
2. Drop every `AdaptationSet` whose `@profiles` does not (explicitly or by inference) contain `ProfA`.
3. Drop every `Representation` likewise.
4. Drop every element/attribute that the document explicitly excludes for `ProfA`, and every extension-namespace item not explicitly
   admitted by `ProfA`.
5. Drop everything `ProfA` says "may be ignored".

An MPD conforms to `ProfA` when `ProfA` is listed, the profile-specific MPD is schema-valid (Annex B), obeys the normative semantics,
and obeys `ProfA`'s restrictions. A Media Presentation conforms when its MPD conforms, every Period still has at least one
Representation in the profile-specific MPD, and the Segments of those Representations obey the profile's Segment restrictions.
Practical consequence for a client: it can treat "may be ignored" items as safe to skip, and must keep a Period playable from what remains.

## Profile identifier table

| Clause | Profile | URN | Family |
|---|---|---|---|
| 8.2 | Full | `urn:mpeg:dash:profile:full:2011` | all |
| 8.3 | ISO BMFF On Demand | `urn:mpeg:dash:profile:isoff-on-demand:2011` | ISO BMFF |
| 8.4 | ISO BMFF Live | `urn:mpeg:dash:profile:isoff-live:2011` | ISO BMFF |
| 8.5 | ISO BMFF Main | `urn:mpeg:dash:profile:isoff-main:2011` | ISO BMFF |
| 8.6 | MPEG-2 TS Main | `urn:mpeg:dash:profile:mp2t-main:2011` | MPEG-2 TS |
| 8.7 | MPEG-2 TS Simple | `urn:mpeg:dash:profile:mp2t-simple:2011` | MPEG-2 TS |
| 8.8 | ISO BMFF Extended Live | `urn:mpeg:dash:profile:isoff-ext-live:2014` | ISO BMFF |
| 8.9 | ISO BMFF Extended On Demand | `urn:mpeg:dash:profile:isoff-ext-on-demand:2014` | ISO BMFF |
| 8.10 | ISO BMFF Common | `urn:mpeg:dash:profile:isoff-common:2014` | ISO BMFF |
| 8.11 | ISO BMFF Broadcast TV | `urn:mpeg:dash:profile:isoff-broadcast:2015` | ISO BMFF |
| 8.12.5 | DASH CMAF Core | `urn:mpeg:dash:profile:cmaf:2019` | CMAF |
| 8.12.6 | DASH CMAF Extended | `urn:mpeg:dash:profile:cmaf-extended:2019` | CMAF |

Every other profile is a subset of Full (p. 180).

## 8.2 Full profile (p. 180)

All features and Segment types of the document are allowed. MPD follows the 7.3 *or* 7.4 rules; Representations follow either the ISO BMFF
formats (6.3 via 7.3) or the MPEG-2 TS formats (6.4 via 7.4). The attributes/elements of 5.2.3.2 may be ignored.

## Constraint matrix for 8.3 to 8.11

Legend: S = shall (hard requirement); I = "may be ignored" (a conforming client may skip the item); - = not addressed.
"7.3" = ISO BMFF MPD/Segment rules, "7.4" = MPEG-2 TS rules (clause 7 is outside this PR; see INDEX-C1.md gaps).

| Constraint | On Demand 8.3 | Live 8.4 | Main 8.5 | TS Main 8.6 | TS Simple 8.7 | Ext Live 8.8 | Ext OnDemand 8.9 | Common 8.10 | Broadcast 8.11 |
|---|---|---|---|---|---|---|---|---|---|
| MPD rules from | 7.3 | 7.3 | 7.3 | 7.4 | 7.4 + 8.6 | 7.3 | 7.3 | 7.3 | 7.3 (+7.3.5 for Segments) |
| 5.2.3.2 items ignorable | I | I | I | I | I | - | - | - | - |
| `MPD@type` | S "static" | free (static allowed for ended live) | free | free | free | free | S "static" | per Period | free |
| Representation without matching `@profiles` | I | I | I | I | I | I | I | I | I |
| `Subset` | I | I | I | I | - | I | I | I | I |
| `xlink:href` elements | I | I | I | (see below) | - | allowed | allowed | allowed | allowed |
| `Period.SegmentList` | S absent | - | - | allowed | allowed | S absent | S absent | per Period | S absent |
| `Period.SegmentTemplate` | S absent | - | - | allowed | allowed | allowed | S absent | per Period | allowed |
| `availabilityStartTime` stable across MPD updates | - | S | - | S | S | - | - | - | - |

Notes on the table:

- 8.6 requires that a presentation be playable without resolving any `xlink:href` on `AdaptationSet` or `SegmentList`; leading Periods
  that use `xlink:href` may be ignored, the first remaining Period must carry an explicit `@start`, and no later Period may use `xlink:href`.
- The "-" cells mean the clause is silent, not that the feature is permitted without limit.

## 8.3 ISO BMFF On Demand (pp. 180-182)

Purpose: one Segment per Representation, Subsegments aligned across an Adaptation Set and starting at a SAP.

- `MPD@type` shall be static; `Subset` may be ignored; neither `Period.SegmentList` nor `Period.SegmentTemplate` may appear.
- Ignorable (clause 8.3.2): an `AdaptationSet` that has `SegmentList`/`SegmentTemplate`; a `Representation` that has
  `SegmentList`/`SegmentTemplate` or has no `BaseURL`; an `AdaptationSet` whose `@subsegmentAlignment` is absent or false; a
  `Representation` whose `@subsegmentStartsWithSAP` is absent, 0 or above 3; one with `@subsegmentStartsWithSAP`=3 when the set has
  several Representations and none shares its `@mediaStreamStructureId`; anything reached through an `xlink:href`.
- Note: complying with 7.3 presupposes `@mimeType` of `video/mp4`, `audio/mp4` or `application/mp4`.
- Segments: ISO BMFF (6.3); exactly one Self-Initializing Media Segment (6.3.5.2) per Representation; every `sidx`/`ssix` precedes
  every `moof`; no Index Segments (a `RepresentationIndex` element or `@indexRange` may still point at the `sidx` inside the Media Segment).

## 8.4 ISO BMFF Live (pp. 182-183)

Purpose: short Segments, template-generated URLs, so a client rarely needs an MPD refresh per Segment; switching at Segment boundaries
by finishing the outgoing Representation then starting the incoming one.

- `SegmentTemplate` shall exist at Period, Adaptation Set or Representation level for each Representation.
- Ignorable: a Representation whose `@startWithSAP` (own or inherited) is absent, 0 or above 3; one with `@startWithSAP`=3 when no peer
  shares its `@mediaStreamStructureId` in a multi-Representation set; an `AdaptationSet` with `@segmentAlignment` false or absent;
  `Subset`; `xlink:href` elements.
- MPD updates must keep `availabilityStartTime` unchanged.
- Segments: each Representation has one Initialization Segment and at least one Media Segment; Media Segments carrying several
  components use the `msix` brand (6.3.4.3); `sidx`/`ssix` before `moof`; no Index Segments.
- `MPD@type` may be static to publish a finished live event as on-demand.

## 8.5 ISO BMFF Main (pp. 183-185)

- Ignorable: Representations with `@startWithSAP` above 3 or inside an Adaptation Set with `@subsegmentStartsWithSAP` above 3; `Subset`;
  `xlink:href` items.
- When `MPD@type` is dynamic, additionally ignorable: sets with `@segmentAlignment` false/absent; Representations with `@startWithSAP`
  absent or 0; those with `@startWithSAP`=3 and no `@mediaStreamStructureId` peer.
- Segments: each track has at least one SAP of type 1-3 in every Subsegment; `sidx`/`ssix` before `moof`; Representations whose
  `@startWithSAP` is absent, 0 or above 3 must use the `msix` brand.

## 8.6 MPEG-2 TS Main (pp. 185-186)

- MPD rules from 7.4. Ignorable: Representations violating 7.4; Representations outside group 0; `Subset`; Representations using
  `SegmentTimeline`.
- `xlink:href` rule as in the notes above; MPD updates keep `availabilityStartTime`.
- Segments follow 7.4/6.4.
- Recommendations (8.6.4): non-conforming Representations may stay as long as the rest is presentable; `SegmentList` is the normal
  case but clients should also handle `SegmentTemplate`; supply Index Segments; do not set `@segmentAlignment` false; use
  `@startWithSAP` 1 or 2 (3 only if `@mediaStreamStructureId` matches across the set).

## 8.7 MPEG-2 TS Simple (pp. 186-187)

A subset of 8.6 designed so a plain MPEG-2 TS engine can play any concatenation of consecutive Segments from one Adaptation Set.

- Everything in 8.6.2/8.6.3 applies. Additionally ignorable: sets with `@subsegmentAlignment` false when an Index Segment exists;
  multi-Representation sets whose `@bitstreamSwitching` is not true. `availabilityStartTime` stable across updates.
- Segments: PSI (including versions) identical across all Representations in the set; one ECM valid per whole Subsegment (or Segment when
  no Index Segment); within an Index Segment a given `sidx` references either media or other `sidx`, never both.
- Recommended: Index Segments and `SegmentTemplate`.

## 8.8 ISO BMFF Extended Live (pp. 187-188)

Extension of 8.4 that keeps remote elements and second-edition features such as events, with extra simplifying limits.

- Period level: `Subset` ignorable; `Period.SegmentList` absent; if a Period has several video Adaptation Sets (`@contentType="video"`),
  at least one carries a Role with scheme `urn:mpeg:dash:role:2011` and value `main`, and all such "main" sets must be perceptually equivalent.
- Adaptation Set level: ignorable unless `SegmentTemplate` is on the set or on every Representation; a set with several
  Representations is ignorable unless `@segmentAlignment` is true and `@startWithSAP` (spelled `@startsWithSAP` in the source text, see
  note below) is 1 or 2.
- Representation level: ignorable if `@mimeType` is not `video/mp4`, `audio/mp4`, `application/mp4` or `text/mp4` (codec/profile
  parameters may be appended); ignorable if `InbandEventStream` is on the Representation or Sub-Representation.
- Segments: ISO BMFF (7.3); `sidx`/`ssix`, if present, before `moof`; no Index Segments.
- Inband events (8.8.4): if an `InbandEventStream` with scheme `urn:mpeg:dash:event:2012` is present in a set, all its Representations
  must carry aligned inband events. MPD-validity-expiration events (5.10.4.2) are essential for correct playback.

> **Spec note:** 8.8.2.3 writes `AdaptationSet@startsWithSAP`; the attribute defined in clause 5 is `@startWithSAP`. Recorded as
> printed; do not silently "fix" when building a validator.

## 8.9 ISO BMFF Extended On Demand (pp. 188-190)

Extension of 8.3 that allows remote elements and second-edition features.

- `MPD@type` shall be static. Period level: no `Period.SegmentList`/`Period.SegmentTemplate`; `Subset` ignorable; same "main" video Role
  rule as 8.8.
- Adaptation Set: ignorable unless every Representation has a `BaseURL`; ignorable if it carries `SegmentList`/`SegmentTemplate`;
  multi-Representation sets ignorable unless `@subsegmentAlignment` is true and `@subsegmentStartsWithSAP` is 1 or 2.
- Representation: same MIME restriction as 8.8; ignorable if it has `SegmentList`/`SegmentTemplate`, lacks `BaseURL`, or is a single
  indexed (or indexed self-initializing) Segment without `SegmentBase@indexRange`.
- Segments: ISO BMFF (6.3); one Segment that is either an Indexed Self-Initializing Media Segment (6.3.5.2) or a Self-Initializing
  Media Segment (6.3.5.1) with an Index Segment present; `sidx`/`ssix` before `moof`; no `emsg` boxes.

## 8.10 ISO BMFF Common (pp. 190-191)

Restricted union of 8.8 and 8.9: an MPD may mix the two, but each Period must follow one of them entirely.

- `MPD@profiles` shall contain the Common URN. Each Period conforms to 8.8.2.2 or to 8.9.2.2; its Segments follow 8.8.3 or 8.9.3 respectively.

## 8.11 ISO BMFF Broadcast TV (pp. 191-193)

Aimed at broadcast TV over broadcast plus broadband, including hybrid unicast/broadcast. Differences from Extended Live:

- One `@timescale` for all Representations of an Adaptation Set.
- Segment durations signalled by `SegmentTimeline` with accurate timing; may sit at Representation level (default at Adaptation Set
  level); `@r` may be -1 (open ended) or >= 0; Segment Sequences (5.3.9.6.4) and Hierarchical Templating (5.3.9.6.5) are allowed.
- Each Representation provides at least one `RandomAccess` element; an Adaptation Set with more than one Representation has at least one
  `Switching` element. Alignment/SAP-start signalling is permitted for backward compatibility but discouraged.
- `@initialization` of `SegmentTemplate` may be an RFC 2397 data URL (8.11.2.5).
- `Period.SegmentList` absent; `Subset` ignorable; `InbandEventStream` only on Adaptation Set level (never on Representation).
- Ignorable: sets without `SegmentTemplate` (own or per Representation), without `RandomAccess`, or (multi-Representation) without
  `Switching` with conforming templates; Representations lacking `RandomAccess`; wrong `@mimeType` as in 8.8.
- Segments (8.11.3): follow 7.3.5. When Segment Sequences are used, the first Segment of a sequence does not carry `dums` as major brand
  in `styp`; all following Segments of that sequence do.
- MPD update protocol (8.11.4), for dynamic MPDs using validity-expiration inband events:
  1. `MPD@publishTime` shall be present; `MPD@minimumUpdatePeriod` should be small, ideally 0.
  2. For any published MPD whose changes go beyond (minimumUpdatePeriod change, `S@r` change, a new `S` entry, or data that left the
     availability window), a new MPD with a new `publishTime` is published and every Segment of every Representation with an
     `InbandEventStream` carries an `emsg` with scheme `urn:mpeg:dash:event:2012`, value 1 or 3, whose message data is the previous
     MPD's `publishTime`.

## 8.12 DASH profiles for CMAF content (pp. 194-209)

### 8.12.1-8.12.2 Overview and CMAF model (pp. 194-197)

- CMAF (ISO/IEC 23000-19) fixes media-object structure independent of manifests; two DASH profiles map it: **Core** (strict timeline
  mapping, a CMAF presentation equals a Period) and **Extended** (relaxed so spliced content need not match Period boundaries exactly).
- Object mapping: CMAF Header -> Initialization Segment; CMAF Segment (and optionally Chunk) -> Media Segment; CMAF Track File ->
  Self-Initializing Media Segment.
- Model symbols used by the clause: for track `k` and fragment `i`: `tf[k,i]` earliest presentation time, `df[k,i] = tf[k,i+1] - tf[k,i]`
  duration, `twc[k,i]` optional wall-clock anchor, chunk `j` of fragment `i` has `tc[k,i,j]`/`dc[k,i,j]`, `C[i]` chunks per fragment,
  `dt[k]` track duration, `CH[k]` header.
- Switching Set rules (8.12.2.4): one media profile for all tracks; one "CMAF Principal Header" `CH*` with the highest decode
  requirements able to initialise the pipeline for the whole set; appending any track header causes no pipeline re-init; equal fragment
  count, equal earliest decode and presentation times, equal durations at each position `i`, equal track duration. These equalities hold for
  Fragments, not necessarily Chunks.
- Timing (8.12.2.5): movie timelines are aligned across tracks but need not start at 0; each track may carry a wall-clock anchor.
- Figure 7 (p. 195, PDF p. 203; our redraw from the rendered page):

```
 CMAF logical media objects   CMAF Track (Header + sequence of Fragments)
 (specified by CMAF)            CMAF Switching Set (stack of Tracks) -> CMAF Aligned Switching Sets -> CMAF Selection Set
 ------------------------------------------------------------------------------------------------
 CMAF addressable media objects:  [CMAF Track]  [CMAF Header]  [CMAF Segment]  [CMAF Chunk]  [CMAF Track File]
 ------------------------------------------------------------------------------------------------
 DASH (specified by DASH):   MPD > Group > Adaptation Set > Representation ;   DASH Segments (point at the addressable objects)
```

  Arrows in the original tie Switching Set -> Adaptation Set (via Group level) and Track -> Representation, and the DASH Segments to
  Header, Segment, Chunk and Track File. No normative content beyond the mapping stated in the text.

### 8.12.3 Mapping principles (pp. 197-200)

- CMAF presentation time 0 maps to Period start; the Period start coincides with the earliest video sample presentation time, and
  `@presentationTimeOffset` carries that offset (derived from baseMediaDecodeTime, composition offset and the track edit list).
- Each CMAF Track becomes a Representation, each Switching Set an Adaptation Set, each Presentation a Period (or a Subset within a Period).
- Non-video samples overlapping the Period start are only partly presented. All Adaptation Sets of one presentation need equal
  *normalised* offsets (offset divided by timescale), though integer values and timescales may differ.
- The video track determines the Period duration. A Period may map only a time span of a presentation provided it starts on a video
  fragment boundary (and, for Core, ends on one).
- Sequential independent presentations map to consecutive Periods where `PD[p] = PST[p+1] - PST[p]`. DASH allows timeline continuity or
  discontinuity and same-initialization or re-initialization across Periods; gaps are signalled with multiple Periods or `SegmentTimeline`.
- Seamless switching (4.5) is available at any Segment/Subsegment start whose SAP type is 1 or 2, i.e. at fragment starts.
- Media profile integration needs: a CMAF media profile definition, a static mapping to `@mimeType`/`@codecs`/`@containerProfiles`, and a
  dynamic mapping from the Principal Header to `@maxWidth`/`@maxHeight` etc. Encryption info is mapped likewise.

### 8.12.4 Common constraints (pp. 200-208)

Representation level (8.12.4.2):

| Rule | Requirement |
|---|---|
| Media Segment | is a CMAF addressable media object (hence also a Delivery Unit Media Segment, 6.3.4.2) |
| Initialization Segment | is a CMAF Header (if present) |
| `@timescale` | equals the track's `mdhd` timescale |
| `@startWithSAP` | if every Segment is a CMAF Fragment and the attribute is present, 1 or 2 |
| `SegmentTimeline` present | one `S` entry per fragment `i`: `@t = tf[k,i]`, `@d = df[k,i]`; `@k` = chunk count when each chunk is addressable; equal-duration neighbours should be merged |
| `@duration` = `dur` present | for each fragment: `((i-1)+0.5)*dur <= tf[k,i+1]-tf[k,1] <= (i+0.5)*dur`; `@startWithSAP` 1 or 2 required |
| Self-Initializing Segment | `@subsegmentStartsWithSAP` 1 or 2; a CMAF Track File; Indexed Self-Initializing Segment (6.3.5.2) with exactly one `sidx` before any `moof` |
| `sidx` fields | `reference_ID` = track id; timescale = `mdhd` timescale; `earliest_presentation_time = tf[k,1]`; `reference_count = N`; one Subsegment per fragment with `reference_type` 0, `referenced_size` >= fragment size, `starts_with_SAP` 1, `subsegment_duration = df[k,i]`, `SAP_type` 1 or 2, `SAP_delta_time` 0 |
| Chunks | when several chunks per Segment, Resynchronization Points should be signalled (5.3.13) |
| Events | event streams relevant to the client should be signalled with `InbandEventStream` (5.10) |

Adaptation Set level (8.12.4.3):

- Each Representation meets the above and the set is a CMAF Switching Set.
- A Principal Header should be signalled by `@initializationSetRef` (an Initialization Set containing `@initialization`) or by
  `@initializationPrincipal`. Absent explicit signalling, the Initialization Segment of the highest-`@bandwidth` Representation may serve.
- `@contentType` follows the Principal Header handler type: `vide` -> video, `soun` -> audio, `subt` -> application, `text` -> text.
  `@mimeType` is `<contentType>/mp4`. `@containerProfiles` should be present and include a structural brand `cmfc` or `cmf2`, preferably
  plus a CMAF media profile brand. `@codecs` comes from the Principal Header sample entry. `@segmentProfiles` may signal object types.
- Protected content: common-encryption rules of 23000-19; a `ContentProtection` with scheme `urn:mpeg:dash:mp4protection:2011` whose value
  is an ISO/IEC 23001-7 scheme (optional `@cenc:default_KID`, equal to the `tenc` default KID when present); and at least one
  `ContentProtection` with a DRM UUID scheme (optional `@cenc:default_KID` and `cenc:pssh`, matching `tenc`/`moov` `pssh`).
- Video: either `@width`/`@height` or `@maxWidth`/`@maxHeight` present, equal to the Principal Header's track header width and height.
  `@frameRate` should be present and, when `trex` `default_sample_duration` is non-zero and `mdhd` timescale present, equals
  `timescale/default_sample_duration` (for example 24000/1001); otherwise codec-config specifics (for example SPS timing) may apply.
- Audio: `@lang` from the `elng` extended language if present, else the `mdhd` language unless it is `und` or `mul`. `kind` boxes with
  scheme `urn:mpeg:dash:role:2011` should become Role descriptors.
- If `@bitstreamSwitching` is true the set follows the CMAF single-initialization constraints. Either `@segmentAlignment` or
  `@subsegmentAlignment` shall be true. Inband event streams may be signalled.

Period level (8.12.4.4):

- A Period may hold several CMAF Presentations, each mapped to one Subset (no Subset means exactly one presentation). Each Switching Set
  maps to exactly one Adaptation Set. Sets in one Subset share the same normalised PTO.
- For a regular Period with a stated duration: normalised PTO is CMAF presentation time zero; for video sets in a Period that is not
  Partially Unavailable (5.3.2.5) the earliest presentation time equals PTO and starts a fragment, so `@eptDelta` is 0 (may be absent), the
  first `S@t` equals PTO, and with a Self-Initializing Segment PTO starts a Subsegment.
- Non-video content before/after the video extent is not presented (or partly presented) between PTO and Period duration; gaps are
  permitted when signalled (8.12.4.5). Early terminated Periods are allowed but discouraged; then `Period@duration` equals the shortest track.

Multi-Period (8.12.4.5):

- A set that signals Period connectivity or continuity (5.2.3.4) to an earlier Period with the same `AdaptationSet@id`: the earlier set's
  Initialization Segments shall be compatible CMAF Headers for this set's switching set. With continuity and a shared Representation `@id`,
  concatenating the two Representations (second without its init) is a conforming CMAF Track.
- Non-video continuity across Periods `i-1` -> `i`: `@pdDelta` of the earlier set equals `@eptDelta` of the later set, both as small as
  possible; a positive `@eptDelta` or negative `@pdDelta` marks a gap and must be signalled; negative values (overlap) should be signalled.
- Without continuity, in a Period that is not Partially Unavailable: if `@duration` or Self-Initializing signalling is used and the start
  offset is not 0, `@eptDelta` is present and negative; with `SegmentTimeline` the first `S@t` does not exceed PTO.
- In a Period that is not a Live-Edge Period: with `@duration` or Self-Initializing signalling and non-zero end offset, `@pdDelta` is
  present and positive; with `SegmentTimeline` whose last `S@r` is non-negative, `@pdDelta` is present and positive whenever it differs
  from (last `S` end time minus PTO, versus Period duration, normalised by `@timescale`).

> **Spec note:** the printed 8.12.4.5 formula for the last-`S` end uses `@t + (@r-1)*@d`; clause 8.12.5 uses the same expression.
> Because `@r` counts *repeats* (an `S` covers `r+1` segments) the end of the last `S` would normally be `@t + (@r+1)*@d`. We record the
> printed form (see the verbatim file) and do not correct it; an implementation of Core/Extended checking should confirm against the
> Ed.6 text before relying on it.

### 8.12.5 DASH CMAF Core Profile (p. 208)

URN `urn:mpeg:dash:profile:cmaf:2019`; may appear at different MPD levels. All of 8.12.4 applies. Additionally, since Period boundaries
coincide with CMAF video track boundaries: for a regular Period with stated duration, video sets in a non-Live-Edge Period have a media
duration (normalised) equal to the Period duration, so `@pdDelta` is 0 (may be absent); with `SegmentTimeline` and a non-negative final
`@r`, `@pdDelta` is the (last-`S`-end minus PTO) versus Period duration difference, normalised; with a Self-Initializing Segment the last
Subsegment ends exactly at the (normalised) Period duration.

### 8.12.6 DASH CMAF Extended Profile (p. 208)

URN `urn:mpeg:dash:profile:cmaf-extended:2019`. All of 8.12.4 applies; no further restriction.

### 8.12.7 Conformance checking (pp. 208-209)

Check the content against the DASH profile format rules and also against CMAF conformance as framed by 8.12.5 or 8.12.6.

## Transcription notes

- Page numbers are printed numbers; PDF page = printed + 8.
- Figure 7 was redrawn from the rendered page (PDF p. 203); see VERIFICATION-C1.md.
- Cross-references to clauses 5.2.3.2, 5.3.9.6.4/5, 5.3.13, 5.10, 6.3, 6.4, 7.3, 7.4 point at material outside this PR except where PR #1516
  covers it ([04-overview.md](04-overview.md) table 1 lists the brand identifiers; [05-3-9-6-segment-timeline.md](05-3-9-6-segment-timeline.md)
  covers 5.3.9.6).
