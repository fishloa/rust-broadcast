---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.3.12 - Initialization Set, Group and Presentation (own-words implementer notes)
clauses: 5.3.12.1-5.3.12.4.2
printed_pages: 99-103
pdf_pages: 107-111
verbatim_source: private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.3.12 Initialization Set, Group and Presentation

## What it defines

A three-level capability hierarchy that tells a client, before it commits to a Media Presentation, which decoder/renderer capabilities it needs.

| Level | Meaning |
|---|---|
| InitializationSet | a bundle of media properties (a subset of the Adaptation Set property list) that holds across Periods |
| InitializationGroup | the minimum needed to play one media type through the whole presentation; every Period must contain an Adaptation Set conforming to at least one of the group's Initialization Sets |
| InitializationPresentation | the minimum for the whole presentation: a client that supports all listed sets and groups should be able to play everything as the provider intends |

Placement: **InitializationSet** is a child of the MPD element, 0..N (Table 3, [`05-3-1-hierarchy.md`](05-3-1-hierarchy.md)).

## Normative behaviour

- If an Initialization Set with given properties is signalled, at least one Adaptation Set in at least one Period shall have those properties.
- `@inAllPeriods` (default true) strengthens that: every Period then has at least one conforming Adaptation Set. A client may select a set at start-up to prepare decryption/decoding/rendering, and then stay consistent by only choosing Adaptation Sets of that set.
- An Initialization Set deliberately specifies fewer properties than the Adaptation Sets that conform to it (examples in the text: audio set with `@codecs` but no `@lang`; video set with `@codecs`, `@maxWidth`, `@maxHeight` while actual sets may differ inside those bounds).
- A set, group or presentation may carry `@profiles` to claim DASH profile / interoperability-point conformance.
- With several Periods there should be at least one Initialization Set per media type (should).
- The group/presentation levels are described only in prose and in Figure 4; no attribute table or syntax for them is printed in this clause.

## Inventory - InitializationSet (Table 27)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| InitializationSet@xlink:href | O | xs:anyURI-style XLink | - | remote element entity holding exactly one InitializationSet |
| @xlink:actuate | OD | onLoad / onRequest | onRequest | XLink processing, see [`05-5-mpd-assembly.md`](05-5-mpd-assembly.md) |
| @xlink:type | fixed | - | simple | in the syntax only |
| @id | M | xs:unsignedInt | - | unique among all InitializationSet and InitializationGroup ids in the MPD |
| @inAllPeriods | OD | xs:boolean | true | whether every Period has a conforming Adaptation Set |
| @contentType | O | RFC6838ContentTypeType | unknown if absent | top-level media type token (RFC 6838) |
| @par | O | RatioType | unknown if absent | picture aspect ratio "a:b"; when `@width`/`@height`/`@sar` of the member Representations exist the ratio must equal width*sarx : height*sary |
| @maxWidth, @maxHeight | O | xs:unsignedInt | unknown | upper bounds over all member Representations, same units as `@width`/`@height` |
| @maxFrameRate | O | FrameRateType | unknown | upper bound, same encoding as `@frameRate` |
| @initialization | O | xs:anyURI | - | URL of an Initialization Segment |
| common attributes/elements | - | RepresentationBaseType | - | see [`05-3-7-common.md`](05-3-7-common.md) |
| Accessibility, Role, Rating, Viewpoint | 0..N | DescriptorType | - | annotation descriptors applying to all member Adaptation Sets |

```text
InitializationSetType = extension of RepresentationBaseType
  sequence: Accessibility* Role* Rating* Viewpoint*  (DescriptorType)
  attributes: xlink:href, xlink:actuate(default onRequest), xlink:type(fixed simple),
              id (unsignedInt, required), inAllPeriods (boolean, default true),
              contentType, par, maxWidth, maxHeight, maxFrameRate, initialization
```

## Figure 4 (example, printed p. 103)

Nine Initialization Sets: IS1-4 video, IS5-7 audio, IS8-9 subtitle. Groups: IG1 = {IS1, IS2}, IG2 = {IS2, IS3}, IG3 = {IS5, IS6}. Presentations: IP1 requires IG1 and IG3 (audio+video); IP2 requires IG1, IG3 and IS9 (adds subtitles). IS9 is flagged `@inAllPeriods` true, so a client supporting IS9 can show subtitles in every Period.

## Quirks

- Prose and figure write `@inAllPeriod` (singular) while the table and syntax define `@inAllPeriods`.
- The paragraph text has typos (doubled "but", "Adaptation Sets" plural after "an", "be present" after "should be at least one"); no semantic effect.
- Table 27 has no sentence forbidding `@xlink:actuate` without `@xlink:href` (the Period table has one).

Verbatim text: see `private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md` section 5.3.12 (printed pp. 99-103).
