<a id="c5-3"></a>

# ISO/IEC 23009-1:2022 clause 5.3 intro and 5.3.1 - hierarchy and the MPD element (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3 and 5.3.1, printed pp. 26-32 (PDF 34-41). Verbatim, page-verified text and the
original Table 3 / XML syntax: `private/specs-verified/dash/23009-1/05-3-1-hierarchy.md`. Use codes: M mandatory, O optional, OD optional with default,
CM conditionally mandatory; element rows show min..max occurrences (N = unbounded). See [`INDEX.md`](INDEX.md).

<a id="c5-3-1"></a>

## 5.3.1 General

<a id="c5-3-1-1"></a>

### 5.3.1.1 Hierarchy overview (p. 26-27)

- MPD contains one or more Period elements.
- Period contains Adaptation Sets (5.3.3); when a set has several content components, each is described on its own (5.3.4). A Period may also hold Preselections (5.3.11) and Subsets (5.3.8) that limit which Adaptation Sets may be presented together.
- Adaptation Set contains Representations (5.3.5). Adaptation Sets, Representations and Sub-Representations share a common attribute/element group (5.3.7).
- Representation consists of Segments (clause 5.15) described by Segment information (5.3.9); a Representation may have Sub-Representations (5.3.5.6/5.3.6) for extracting part of it. Segments split into Subsegments (6.2.3.2).
- Labels for each level: 5.3.10.

Element tree (own outline, Table 3 order; occurrences in brackets):

```text
MPD
 +- ProgramInformation [0..N]      (5.7)
 +- BaseURL [0..N]                 (5.6)
 +- Location [0..N]                (where the MPD can be fetched; Annex A.11)
 +- PatchLocation [0..N]           (MPD patch, 5.15)
 +- ServiceDescription [0..N]      (Annex K)
 +- InitializationSet [0..N]       (5.3.12)
 +- InitializationGroup [0..N]     (id list; 5.3.12)
 +- InitializationPresentation [0..N]
 +- ContentProtection [0..N]       (5.8.1, 5.8.4.1; must carry @refId at this level)
 +- Period [1..N]                  (5.3.2)
 +- Metrics [0..N]                 (5.9)
 +- EssentialProperty [0..N], SupplementalProperty [0..N]   (5.8.4.8, 5.8.4.9)
 +- UTCTiming [0..N]               (5.8.4.11; document order = author preference)
 +- LeapSecondInformation [0..1]   (5.13)
 +- (any element from another namespace, lax)
```

<a id="c5-3-1-2"></a>

### 5.3.1.2 MPD semantics (Table 3, p. 27-31)

| Name | Use | Type | Default | Gloss (own words) |
|---|---|---|---|---|
| `@id` | O | string | - | Identifier of the presentation; should be unique where published. Absent means no internal id (the MPD URL can stand in). |
| `@profiles` | M | list of profile URNs/URLs | - | Profiles the presentation conforms to (clause 8). Content must match the RFC 6381 section 4.5 simple or fancy profile productions without the surrounding quotes; ids use the restricted URI form of 8.1. |
| `@type` | OD | `static` / `dynamic` | `static` | Static: every Segment is available from `@availabilityStartTime` to `@availabilityEndTime`. Dynamic: availability times differ per Segment (see 5.3.9.5.3) and the MPD may change. Static is typical for on-demand, dynamic for live. |
| `@availabilityStartTime` | CM (required if dynamic) | dateTime (UTC) | - | Dynamic: anchor for computing each Segment's earliest availability. Static: availability start for all Segments; if absent, everything is available when the MPD is. |
| `@publishTime` | OD (required if dynamic) | dateTime | - | Wall-clock time the MPD was generated at the origin; an MPD with a later value must be a valid update of an earlier one (5.4). |
| `@availabilityEndTime` | O | dateTime | unknown | Latest Segment availability end time of any Segment. |
| `@mediaPresentationDuration` | O | duration | unknown | Total presentation length. Required when neither `@minimumUpdatePeriod` nor the last Period's `@duration` is present. |
| `@minimumUpdatePeriod` | O | duration | MPD never changes | Shortest interval between possible MPD changes, i.e. the validity time after a fetch (5.4). Must not appear unless `@type` is dynamic. |
| `@minBufferTime` | M | duration | - | The common time window used to define Representation `@bandwidth` (5.3.5.2, 5.3.5.4). |
| `@timeShiftBufferDepth` | O | duration | infinite | Guaranteed time-shift depth of the smallest buffer among Representations (dynamic only; undefined for static). |
| `@suggestedPresentationDelay` | O | duration | none (client chooses) | Dynamic only: suggested fixed delay behind each access unit's presentation time (7.2.1). Ignorable for static. |
| `@maxSegmentDuration` | O | duration | longest Segment in this MPD | Upper bound on any Segment duration, including future MPD updates. |
| `@maxSubsegmentDuration` | O | duration | same as max Segment duration | Upper bound on any Media Subsegment. |
| `PatchLocation` | 0..N | URI + attrs | - | Where an MPD patch is served. If present, `MPD@id` and `MPD@publishTime` must be present. Undefined/ignorable if static or no `@minimumUpdatePeriod`. Any listed location may be used. |
| `InitializationGroup` | 0..N | uint list | - | Ids of Initialization Sets of one content type; promises every Period has an Adaptation Set matching one of them. |
| (cont.) |||| Attributes: `@id` (M, unsigned int, unique among all Initialization Set and Group ids), `@profiles` (O, subset of MPD profiles, default = MPD's), `@contentType` (O). |
| `InitializationPresentation` | 0..N | uint list | - | Combination of Initialization Sets/Groups forming a complete presentation; a client supporting all of them should play everything. `@id` (M, unique among Initialization Presentation ids), `@profiles` (O), `@contentType` (O). |
| `InitializationSet` | 0..N | see 5.3.12 | - | An initialization usable for one media type; at least one Period must have an Adaptation Set playable with it. |
| `ServiceDescription` | 0..N | Annex K | - | Provider's expectations on consumption; must be a valid service description per Annex K.3/K.4. |
| `ContentProtection` | 0..N | 5.8 | - | Protection schemes in use; must include `@refId` at MPD level. |
| `Location`, `ProgramInformation`, `BaseURL`, `Metrics`, `Period` (1..N), `EssentialProperty`, `SupplementalProperty`, `UTCTiming`, `LeapSecondInformation` | see tree | - | - | Cross-references listed in the tree above. |

<a id="c5-3-1-3"></a>

### 5.3.1.3 Schema types (p. 31-32)

Facts from the syntax block (the Annex B XSD governs; see 5.2.2):

- `MPDtype`: a sequence of the children in the tree above, then `xs:any` from other namespaces; attributes as in the table; `xs:anyAttribute` from other namespaces. `Location` is `xs:anyURI`; `Period` has no minOccurs (so 1..N).
- `PresentationType`: string enumeration `static` | `dynamic`.
- `UIntVWithIDType`: list of `xs:unsignedInt` (`UIntVectorType`) with attributes `@id` (unsignedInt, required), `@profiles` (`ListOfProfilesType`), `@contentType` (`RFC6838ContentTypeType`). Used for `InitializationGroup` and `InitializationPresentation`.
- `ListOfProfilesType`: string restricted by a pattern of one or more URN-or-URL items separated by commas.
- Ed.6 differences: see [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

## Transcription notes (spec quirks)

- 5.3.1.1 spells subclause "5.15" as "sublause 5.15" (p. 26).
- Table 3 `@type`: the sentence on `@availabilityStartTime` for static has a mismatched closing quote in print (p. 27); `@publishTime` is coded OD with a "shall be present for dynamic" remark and no default value (p. 27).
- The normative timing formulas for the MPD-level attributes are not in 5.3.1.2; they are in clause 7.2.1 and Annex A, outside the transcribed range ([`VERIFICATION.md`](VERIFICATION.md), gaps).
