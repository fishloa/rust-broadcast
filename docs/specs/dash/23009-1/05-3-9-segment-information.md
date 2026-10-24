<a id="c5-3-9"></a>

# ISO/IEC 23009-1:2022 clauses 5.3.9 to 5.3.9.3 - Segments and Segment information (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.9 general, 5.3.9.2 Segment base information and 5.3.9.3 Segment list, printed pp. 71-82 (PDF 79-90).
Verbatim, page-verified text, Tables 16-19 and XML syntax: `private/specs-verified/dash/23009-1/05-3-9-segment-information.md`.
Segment templates are in [`05-3-9-4-segment-template.md`](05-3-9-4-segment-template.md), the derivation of Segment URLs/times in
[`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md). See [`INDEX.md`](INDEX.md).

<a id="c5-3-9-1"></a>

## 5.3.9.1 General rules for Segment information (p. 71-72)

- A Segment is either (a) an HTTP-URL from the MPD (absolute-URI per RFC 3986 section 4.3, scheme `http` or `https`), optionally restricted by a single contiguous byte-range-spec (RFC 2616 14.35.1 as
  cited in print), or (b) embedded in the MPD as a data URL (RFC 2397), allowed only if the profile permits it.
- Every Segment reached through an HTTP-URL has a Segment availability interval: a wall-clock window (availability start time to end time) during which the URL works.
- Segment information is given by `BaseURL`, `SegmentBase`, `SegmentTemplate` and/or `SegmentList`; it tells location, availability and properties of the Initialization, Media, Index and Bitstream Switching Segments of one Representation.
- Placement and inheritance:
  - The three Segment elements may sit on Representation, AdaptationSet or Period (the latter two give defaults).
  - At each level at most one of SegmentBase, SegmentTemplate, SegmentList may be present.
  - If SegmentTemplate or SegmentList is present at one level, the *other* of the two may not appear at any lower level.
  - An element inherits attributes/children from the same element at a higher level; on conflict the lower level wins.
- Each Representation ends up with exactly one of: one `SegmentList`; one `SegmentTemplate`; or one-or-more `BaseURL` with at most one `SegmentBase` and neither of the other two (single-Segment style). BaseURL may also accompany SegmentList or SegmentTemplate, in which case 5.6 processing applies.
- All three elements are built on the `SegmentBase` type; SegmentTemplate and SegmentList additionally share the multiple-Segment attributes (Table 17).

<a id="c5-3-9-2"></a>

## 5.3.9.2 Segment base information

<a id="c5-3-9-2-1"></a>

### 5.3.9.2.1 When SegmentBase suffices (p. 71-72)

`SegmentBase` alone is enough if and only if the Representation has a single Media Segment and its URL is in `BaseURL`. For several Media Segments use SegmentList or SegmentTemplate, which add the Table 17 fields. With more than one Media Segment exactly one of `@duration` and
`SegmentTimeline` must be present (never both). The addresses in this machinery are of type `URLType` (Table 18).

<a id="c5-3-9-2-2"></a>

### 5.3.9.2.2 Semantics

Table 16 (SegmentBase and the Segment Base Information type, p. 72-75):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@timescale` | O | unsigned int (units/s) | 1 if absent at every level | Tick rate for deriving real durations in Segment information; typically a media clock (or a positive integer multiple). |
| `@presentationTimeOffset` | O | unsigned long | 0 | Presentation time, in `@timescale` units, of the stream sample that is presented at Period start (offset of the Representation relative to Period start). Seconds = value / timescale. |
| `@eptDelta` | O | integer | unknown (a profile may require 0) | Earliest presentation time of the Representation's first Media Segment minus `@presentationTimeOffset`, in timescale units. |
| (cont.) |||| For self-initializing Media Segments, the offset identifies the first Subsegment and `@eptDelta` is the difference between offset and that Subsegment's earliest presentation time. |
| (cont.) |||| Negative = overlap before the Period; positive = gap. |
| `@pdDelta` | O | integer | unknown | Representation presentation duration minus Period duration, in timescale units. |
| (cont.) |||| For self-initializing Segments, offset + Period duration identifies the last Subsegment and `@pdDelta` follows from that last Subsegment's duration. Negative = gap at the end; |
| (cont.) |||| positive = overlap. |
| `@presentationDuration` | O | unsigned long | unknown (present until Period end) | Presentation duration of the Representation in the Period, in timescale units. Offset (or 0) plus this value is the last presentation time to present. |
| `@timeShiftBufferDepth` | O | duration | MPD-level value | Guaranteed time-shift depth for this Representation in dynamic presentations; must not be smaller than the MPD-level value; undefined for static. |
| (cont.) |||| A depth above the MPD value means not all Representations are necessarily available for switching. |
| `@indexRange` | O | single byte range `a-b` | unknown | Byte range holding the Segment Index in every Media Segment; one contiguous range-spec (RFC 7233 2.1). |
| `@indexRangeExact` | OD | boolean | `false` | True: for all Segments, the data outside the `@indexRange` prefix is sufficient to access every access unit of all streams. Must not appear without `@indexRange`. |
| `@availabilityTimeOffset` | O | double (seconds) | none defined | How much earlier than computed the Segments become available - gives the *adjusted* availability start time (5.3.9.5). `INF` means all Segments are available from `MPD@availabilityStartTime`. |
| `@availabilityTimeComplete` | O | boolean | true | Whether Segments are complete at the adjusted availability start time; ignored when no `@availabilityTimeOffset` at any level. False means a Segment is reachable before it is complete. |
| `Initialization` | 0..1 | URLType | - | URL (+ optional range) of the Initialization Segment. |
| `RepresentationIndex` | 0..1 | URLType | - | URL (+ optional range) of the Representation Index Segment. |
| `FailoverContent` | 0..1 | 5.3.9.7 | - | Times where content was replaced by failover content (for example an encoder error). |

Table 17 (MultipleSegmentBaseInformation, p. 76), inherited by SegmentTemplate and SegmentList:

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@duration` | O | unsigned int | - | Constant approximate Segment duration in timescale units; all Segments in the Representation have it except possibly a much shorter last Segment of the Period. See 5.3.9.5.3. |
| `@startNumber` | O | unsigned int | see 5.3.9.5.3 | Number of the first Media Segment of the Representation in the Period. |
| `@endNumber` | O | unsigned int | inferred from Period duration | Number of the last Media Segment. |
| `SegmentTimeline` | 0..1 | 5.3.9.6 | - | Arbitrary per-Segment durations. |
| `BitstreamSwitching` | 0..1 | URLType | - | URL (+ range) of the Bitstream Switching Segment. |

Table 18 (any element of type `URLType`, p. 77):

| Name | Use | Type | Gloss |
|---|---|---|---|
| `@sourceURL` | O | anyURI | Absolute http(s) URI or a relative-reference (RFC 3986 4.3 / 4.2). If absent, the `BaseURL` is used as the source and `@range` must be present. |
| `@range` | O | single byte range | Restricts the URL to that range (RFC 7233 2.1); if absent, the whole resource is meant. |

<a id="c5-3-9-2-3"></a>

### 5.3.9.2.3 Type outline

```text
SegmentBaseType : { Initialization?, RepresentationIndex?, FailoverContent?, (any other) }
   attrs: @timescale uint  @eptDelta int  @pdDelta int  @presentationTimeOffset ulong  @presentationDuration ulong
          @timeShiftBufferDepth duration  @indexRange range  @indexRangeExact bool=false
          @availabilityTimeOffset double  @availabilityTimeComplete bool
MultipleSegmentBaseType : SegmentBaseType + { SegmentTimeline?, BitstreamSwitching? }
   attrs: @duration uint  @startNumber uint  @endNumber uint
URLType : @sourceURL anyURI  @range range    SingleRFC7233RangeType = string matching ([0-9]*)(\-([0-9]*))?
```

Ed.5 versus Ed.6: `@startNumber`/`@endNumber` widen from unsignedInt to unsignedLong in the Ed.6 schema, which also adds `@endSubNumber`, `@k`, `@tolerance` ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

<a id="c5-3-9-3"></a>

## 5.3.9.3 Segment list

<a id="c5-3-9-3-1"></a>

### 5.3.9.3.1 Overview (p. 78)

`SegmentList` carries an ordered list of `SegmentURL` entries for consecutive Segments; each entry may give a Media Segment URL with an optional byte range, and optionally an Index Segment.

<a id="c5-3-9-3-2"></a>

### 5.3.9.3.2 Semantics (Table 19, p. 78-80)

`SegmentList` extends MultipleSegmentBaseInformation (Table 17) with optional `@xlink:href` (remote entity holding one or more SegmentList elements) and `@xlink:actuate` (`onLoad`/`onRequest`, default `onRequest`); with xlink everything becomes optional. Child `SegmentURL` (0..N):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@media` | O | anyURI | - | Media Segment URL (absolute http(s) URI or relative-ref), used with `@mediaRange`. If absent, `BaseURL` is mapped onto it and `@mediaRange` must be present. |
| `@mediaRange` | O | single byte range | whole resource | Range inside the `@media` resource that is the Media Segment. |
| `@index` | O | anyURI | see rule | Index Segment URL used with `@indexRange`. Absent with `@indexRange` absent: no Index Segment info. Absent with `@indexRange` present: `@media` is used as the index resource; if `@media` is also absent, `BaseURL` is used and `@indexRange` must be present. |
| `@indexRange` | O | single byte range | whole `@index` resource | Range of the Index Segment within `@index`; when `@index` is absent it is the byte range of the Segment Index inside the Media Segment. |

<a id="c5-3-9-3-3"></a>

### 5.3.9.3.3 Type outline

`SegmentListType` extends `MultipleSegmentBaseType` with `SegmentURL*` and xlink attributes (`href`, `actuate` default `onRequest`, `type` fixed `simple`, `show` fixed `embed`). `SegmentURLType`:
attributes `@media`, `@index` (anyURI), `@mediaRange`, `@indexRange` (single range), any-other-namespace attributes and children.
Unchanged in Ed.6 for `SegmentListType` and `SegmentURLType`.

## Transcription notes (spec quirks)

- 5.3.9.1 states the data-URL rule with "RFC 2397"; the byte-range citation names RFC 2616 14.35.1 here but RFC 7233 2.1 in the Tables 16, 18 and 19 (p. 71-80); implementers should treat the grammar (`byte-range-spec` restricted to one contiguous range) as the same.
- `@startNumber`, `@endNumber` and `@duration` are 32-bit in the Ed.5 clause snippets (`xs:unsignedInt`); live services with long-running numbering may need the Ed.6 width (see the schema diff).
