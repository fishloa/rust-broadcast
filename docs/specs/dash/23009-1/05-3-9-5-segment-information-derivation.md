<a id="c5-3-9-5"></a>

# ISO/IEC 23009-1:2022 clause 5.3.9.5 - deriving Segment information (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.9.5, printed pp. 84-87 (PDF 92-95). Verbatim, page-verified text:
`private/specs-verified/dash/23009-1/05-3-9-5-segment-information-derivation.md`. Notation below: `k` = 1-based position of a Media Segment in the Representation within its Period,
`startNumber` = `@startNumber` (1 if absent), `AST` = `MPD@availabilityStartTime`, `PeriodStart` per [`05-3-2-period.md`](05-3-2-period.md#c5-3-2-1). See [`INDEX.md`](INDEX.md).

<a id="c5-3-9-5-1"></a>

## 5.3.9.5.1 What Segment information yields (p. 84)

From SegmentBase/SegmentList/SegmentTemplate the client derives: which of Initialization, Index and Bitstream Switching Segments exist; the HTTP-URL (and byte range) of every accessible Segment, or an embedded data URL; the full set of valid Segment URLs the MPD declares;
for dynamic MPDs, each Segment's availability start and end time; and an approximate Media Presentation start time of each Media Segment within its Period. Reference resolution (5.6.4) and base URL selection (5.6.5) apply to every URL produced.

<a id="c5-3-9-5-2"></a>

## 5.3.9.5.2 Initialization Segment (p. 84)

- At most one per Representation. It exists when `SegmentBase.Initialization`, `SegmentList.Initialization`, `SegmentTemplate.Initialization` or `SegmentTemplate@initialization` is present (URL + optional range, or a template).
- If none is present, every Media Segment of the Representation must be self-initializing.
- Dynamic MPDs: Initialization Segment availability start = `AST + PeriodStart`; its availability end = the largest availability end time of any Media Segment of the Representation.
- Data format of what the URL returns: clause 6.2.2.

<a id="c5-3-9-5-3"></a>

## 5.3.9.5.3 Media Segments: URLs, times and availability (p. 85-87)

A Representation with more than one Media Segment has a consecutive list of them, explicit (`SegmentList`) or implicit (`SegmentTemplate`). Each entry has a URL (plus optional range), a
number/position, an MPD start time (approximate presentation start inside the Period) and an MPD duration (approximate length).
The MPD times are approximate and need not equal exact Media Presentation time (relation: clause 7.2.1).

**URL for position k**

- `SegmentTemplate` (contained or inherited) with `$Number$`: the spec prints the replacement as `(k-1) + (kstart-1)` with `kstart = @startNumber` or 1 (p. 85), which gives `startNumber - 1` for the first Segment. Use `startNumber + k - 1`, which is what the `@duration` rule prints as `(k-1) + NumberStart` (p. 86) and what the committed real captures show (quirk note below; `RECHECK-P0.md` D2).
- `SegmentTemplate` (contained or inherited) with `$Time$`: this bullet says "the MPD start time of this segment", but with a `SegmentTimeline` the MPD start time is `S@t - PTO` (5.3.9.6.1) while Table 21 (5.3.9.4.4) and 5.3.9.6.1 say the substituted value is the timeline's `@t` / earliest presentation time itself. The three statements disagree whenever `PTO != 0`; substitute the raw `S@t` (see `RECHECK-P0.md` D3 and [`05-3-9-4-segment-template.md`](05-3-9-4-segment-template.md)).
- `SegmentList`: the first listed Segment of the Period has number `SegmentList@startNumber` (1 if absent); several SegmentList elements in one Representation must continue with consecutive numbers.
- Neither: exactly one Media Segment, its URL from `BaseURL`, with an optional `SegmentBase`.

**MPD start time and duration**

- No `@duration` and no `SegmentTimeline`: exactly one Media Segment; start 0; duration derived like the last Segment (below).
- `@duration` present: `MPDstart(Number) = (Number - NumberStart) * @duration` (timescale units), where `Number = (k - 1) + NumberStart`; duration is `@duration` except for the last Segment.
- `@duration` absent, `SegmentTimeline` present: the rules of 5.3.9.6 give start and duration (see [`05-3-9-6-segment-timeline.md`](05-3-9-6-segment-timeline.md)).
- **Only or last Segment of a Representation in a Period**: the MPD must carry enough information to know the Period duration (`MPD@mediaPresentationDuration`, `Period@duration`, or the next Period's
  `@start`). The author should always provide a Segment covering the Period's full duration, even an empty one.
  If the number of Segments differs from the nominal one, add `@endNumber` (number of the last Segment); it should differ from the nominal value by at most 1, and a Segment numbered above `@endNumber` is invalid.

**Availability (dynamic MPDs only)**

- `SegAvailStart(k) = AST + PeriodStart + MPDstart(k) + MPDduration(k)` - the segment becomes available once it is complete. Note in the spec: adding the Segment's own duration means a Representation with shorter Segments gets its first Segment available earlier, which can be used to cut latency.
- `SegAvailEnd(k) = SegAvailStart(k) + MPDduration(k) + @timeShiftBufferDepth` of the Representation.
- **availabilityTimeOffset**: `ATO` = sum of all `@availabilityTimeOffset` values on every level processed to build the Segment's URL (Segment information and BaseURL levels). Adjusted availability start = `SegAvailStart - ATO`.
  If `@availabilityTimeComplete` is false at any level, the Segment may not be complete at the adjusted time and nothing is promised about how it becomes available (gradual availability signalling: 5.3.12.1).
- Coverage rule for authors: the MPD must list every Segment whose availability start is earlier than both the end of the presentation and (latest time this MPD version is on the server + `@minimumUpdatePeriod`).
- Format of what a Media Segment URL returns: clause 6.2.3.

<a id="c5-3-9-5-4"></a>

## 5.3.9.5.4 Index Segments (p. 87)

- Explicit Index Segment information is signalled by: one `RepresentationIndex` (index for the whole Representation); or `@index` and/or `@indexRange` on a `SegmentList.SegmentURL`; or `SegmentTemplate@index`.
- With `SegmentTemplate@index`, if the template contains `$Number$` or `$Time$` it is processed like the media template with the number of the corresponding Media Segment; without them the attribute is a reference to a Representation Index.
- `@indexRange` can also locate an index inside a Media Segment where the format allows: then `@index` must be absent and the range must lie fully within any range given for the Media Segment.
- Index Segment availability equals that of the Media Segments they correspond to. Returned data format: clause 6.2.4.

<a id="c5-3-9-5-5"></a>

## 5.3.9.5.5 Bitstream Switching Segment (p. 87)

At most one per Representation; relevant only when `@bitstreamSwitching` is true, where it may enable a conforming Segment track across Representations. Presence is signalled by `BitstreamSwitching`
or `SegmentTemplate@bitstreamSwitching`. If neither is present while the flag is true, there are no Bitstream Switching Segments.
Its availability time equals that of the Initialization Segment (5.3.9.5.2). Returned data format: clause 6.2.5.

## Transcription notes (spec quirks)

- **$Number$ formula inconsistency (5.3.9.5.3, p. 85-86).** The `$Number$` bullet (p. 85) prints the replacement as "(k-1) + (kstart-1)" with kstart the `@startNumber` (or 1), while the `@duration` bullet on the
  next page (p. 86) writes the segment number as "(k-1) + NumberStart". The two differ by exactly 1 (for k = 1 the first yields startNumber - 1, the second startNumber).
  The second is consistent with the start-time formula and with `SegmentList@startNumber` being the number of the first Segment; treat the first as a misprint. Real output confirms it: `fixtures/dash/manifest.mpd` has `startNumber="1"` and its first Segment is `chunk-stream0-00001.m4s` with `tfdt` 0, which the `(kstart-1)` reading would call number 0; the same holds for `fixtures/dash/timeline-gpac/` (GPAC, `startNumber="1"`, `$Number$`, first Segment `seg-1-1.m4s`, `tfdt` 0).
- Cross-reference "5.3.12.1" (gradual Segment availability) and "6.2.x" data-structure pointers are outside the transcribed range.
