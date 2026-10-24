<a id="c5-3-2"></a>

# ISO/IEC 23009-1:2022 clause 5.3.2 - Period (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.2, printed pp. 33-39 (PDF 41-47). Verbatim, page-verified text, Table 4 and the XML syntax:
`private/specs-verified/dash/23009-1/05-3-2-period.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-2-1"></a>

## 5.3.2.1 Period kinds and PeriodStart derivation (p. 33-34)

Three kinds of Period exist: regular, early available, early terminated. Decision procedure, applied per Period in document order:

1. `@start` present: the Period is regular or early terminated and PeriodStart = `@start`.
2. `@start` absent and the previous Period has `@duration`: regular/early terminated; PeriodStart = previous PeriodStart + previous `@duration`.
3. `@start` absent, first Period, `MPD@type` static: PeriodStart = 0.
4. `@start` absent, and (previous Period has no `@duration` or this is the first Period), `MPD@type` dynamic: the Period is an Early Available Period.
5. `@duration` present and (the next Period has `@start` or `MPD@minimumUpdatePeriod` is present): the Period is an Early Terminated Period.

Duration semantics:

- Regular and early terminated Periods: PeriodStart is the media time that elapses after all earlier Periods, relative to the first Period's start. A Period lasts until the next PeriodStart, or until the end of the
  presentation for the last one. For a regular Period the presentation duration equals the gap to the next PeriodStart (or to `MPD@mediaPresentationDuration` for the last Period). For an early terminated Period the duration is its own `@duration`.
- Early Available Periods announce initialisation of non-media data before media exists. They must not precede any regular Period in the document; whatever resources they announce must be available;
  they are not Periods of the presentation until an MPD update supplies a PeriodStart, and an update may delete them while none is assigned.
- To avoid resolving a remote (xlink) Period just to learn the timeline (for example on a seek), `Period@start` or the previous Period's `@duration` should be present.

Formula view (own notation): `PeriodStart[i] = start[i]` if given; else `PeriodStart[i-1] + duration[i-1]`; first Period of a static MPD defaults to 0.

<a id="c5-3-2-2"></a>

## 5.3.2.2 Period semantics (Table 4, p. 34-36)

Unless xlink is used, the Use codes hold; with `@xlink:href` every attribute becomes optional and every element minOccurs 0.

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@xlink:href` | O | URI | - | Reference to a remote entity that is empty or holds one or more top-level Period elements. |
| `@xlink:actuate` | OD | `onLoad` / `onRequest` | `onRequest` | When to resolve; must not appear without `@xlink:href`. |
| `@id` | O | string | none | Unique within the presentation. Mandatory (and stable across MPD updates) when the MPD is dynamic. Remote entities need ids unique in the presentation; |
| (cont.) |||| a generator unaware of them can use UUIDs, and if the resolver returns the same generated id every time the entity stays cacheable. |
| `@start` | O | duration | see 5.3.2.1 | PeriodStart: the anchor for each Segment's MPD start time and for access-unit presentation times; with `MPD@availabilityStartTime` it yields availability times of dynamic presentations (5.3.9). |
| `@duration` | O | duration | see 5.3.2.1 | Used to compute the next Period's start. |
| `@bitstreamSwitching` | OD | boolean | `false` | True is shorthand for `AdaptationSet@bitstreamSwitching=true` on every set of the Period, none of which may then say false. |
| `BaseURL` | 0..N | - | - | Reference base and alternative-location choice (5.6). |
| `SegmentBase`, `SegmentList`, `SegmentTemplate` | 0..1 each | - | - | Period-level defaults for Segment information, overridden by the AdaptationSet and then Representation versions (5.3.9). |
| `AssetIdentifier` | 0..1 | descriptor | - | Marks the Period as part of an asset. |
| `EventStream` | 0..N | - | - | Event stream (5.10.2). |
| `ServiceDescription` | 0..N | - | - | Per-Period consumption expectations (Annex K); if any is present it overrides all MPD-level service description of the same scope. |
| `ContentProtection` | 0..N | - | - | Protection info; must carry `@refId` at this level. |
| `AdaptationSet` | 0..N | - | - | At least one is required unless the Period's `@duration` is zero. |
| `Subset` | 0..N | - | - | Restricts joint presentation of sets (5.3.8). |
| `SupplementalProperty` | 0..N | descriptor | - | Optional information for client optimisation. |
| `EmptyAdaptationSet` | 0..N | AdaptationSet-typed | - | A set with no Representation and no xlink; allowed only together with an Essential Descriptor whose scheme is `urn:mpeg:dash:mpd-as-linking:2015`. |
| `GroupLabel` | 0..N | label | - | Summary label for a group of Labels (5.3.10). |
| `Preselection` | 0..N | - | - | A combination of Adaptation Sets forming one experience (5.3.11). |

Schema facts: `PeriodType` is a sequence of the children above (document order as in the table) plus any-other-namespace, with xlink attributes `href`, `actuate` (default `onRequest`),
`type` (fixed `simple`), `show` (fixed `embed`) and `@id` (string), `@start`/`@duration` (`xs:duration`), `@bitstreamSwitching` (boolean, default false). Ed.6 differences: [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

<a id="c5-3-2-3"></a>

## 5.3.2.3 Element tree

```text
Period [@xlink:href @xlink:actuate @id @start @duration @bitstreamSwitching]
 +- BaseURL*  +- SegmentBase?  +- SegmentList?  +- SegmentTemplate?
 +- AssetIdentifier?  +- EventStream*  +- ServiceDescription*  +- ContentProtection*
 +- AdaptationSet*  +- Subset*  +- SupplementalProperty*  +- EmptyAdaptationSet*
 +- GroupLabel*  +- Preselection*  +- (other-namespace any)
```

<a id="c5-3-2-4"></a>

## 5.3.2.4 Multi-Period content, continuity and connectivity (p. 37-39)

Reasons the spec gives for multiple Periods: splicing (ad insertion), resyncing segment numbering where durations are irregular, adding or removing Representations, Adaptation Sets or CDNs, and signalling shorter Segments.
Normally Period boundaries are resync points, but content may continue across them and then playout is expected to be continuous.

- **Association.** Two Adaptation Sets in different Periods are associated when both Periods carry equivalent Asset Identifiers and both sets have the same `@id`. Meaning: logical continuation of that component (a client may keep playing it into the new Period, e.g. after an ad Period).
- **Period-continuous** needs all of: associated; for each set, `@presentationTimeOffset` plus the total presentation duration of its Representations equals the associated set's `@presentationTimeOffset` in the next Period;
  and for same-`@id` Representations, functionally equivalent Initialization Segments such that Initialization Segment + all Media Segments of period 1 + all Media Segments of period 2 form a
  conforming Segment track (4.5.4) for the `@mimeType` of the first Period's Representation; the later Period's `@mimeType` must equal one of the first Period's.
- Signal it with a SupplementalProperty on the Adaptation Set, scheme `urn:mpeg:dash:period-continuity:2015`, whose `@value` is the `@id` of a Period in the MPD (the preceding one), with the same `AdaptationSet@id` in both Periods. MPDs should signal this when Periods share Asset Identifiers.
- **Period-connected**: same conditions except timestamps may be discontinuous across the boundary, compensated by `@presentationTimeOffset` (wrap-around, encoder reset, splice); Initialization
  Segments still equivalent. Signal with scheme `urn:mpeg:dash:period-connectivity:2015`. Continuity implies connectivity. Client behaviour: Annex A.9.

<a id="c5-3-2-5"></a>

## 5.3.2.5 Special Periods in live services (p. 39)

- **Partially Unavailable Period**: in a dynamic presentation, a Period whose `availabilityStartTime + Period@start` is earlier than `publishTime - timeShiftBufferDepth`; some of its Segments have left the time-shift window.
- **Live-Edge Period**: the newest regular Period of a dynamic presentation whose duration is unknown and where at least one Segment's availability start time is later than `@publishTime`; the length of not-yet-available Segments may be unknown.
- One Period can be both.

## Transcription notes (spec quirks)

- No wrong cross-references were found in this clause. In 5.3.2.1 the list is the authoritative derivation; the later text in 5.3.2.1 about early terminated Periods repeats condition 5 in prose.
- The Table 4 ServiceDescription row points at "Table 3 in 5.3.1.3" for details; the semantics table is in 5.3.1.2 (printed p. 35).
