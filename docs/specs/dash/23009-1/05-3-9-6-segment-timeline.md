<a id="c5-3-9-6"></a>

# ISO/IEC 23009-1:2022 clause 5.3.9.6 - Segment timeline (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.9.6, printed pp. 88-92 (PDF 96-100). Verbatim, page-verified text, Table 22 and XML syntax:
`private/specs-verified/dash/23009-1/05-3-9-6-segment-timeline.md`. Notation: `PTO` = `@presentationTimeOffset`, all times in `@timescale` units. See [`INDEX.md`](INDEX.md).

<a id="c5-3-9-6-1"></a>

## 5.3.9.6.1 What the timeline is and how it expands (p. 88)

`SegmentTimeline` states the earliest presentation time and duration of each Segment; it is the alternative to `@duration` (the two are mutually exclusive, 5.3.9.2.1) and buys: arbitrary durations,
accurate durations for one media stream, signalling of timeline discontinuities for which a Representation holds no data, and Segment sequences (5.3.9.6.4; only if the profile permits).

**Structure.** A list of `S` entries, each describing a run of contiguous Segments of identical MPD duration, written in the order of the Segments (textual order must equal numbering and time order).

**Expansion algorithm (restated).** Keep a running `time` and `number`. For each `S` in order:

1. Start time of the run: if `@t` is present, `time = @t` (it may exceed the previous end, which signals a discontinuity, and must never be smaller than the previous end); if absent, the first `S` starts at 0 and later ones at `previous.t + previous.d * (previous.r + 1)`.
   The MPD start time (relative to the Period) of the first Segment of the run is `time - PTO`.
2. Number of the run's first Segment: `@n` if present (must be at least one greater than the number of preceding `S` elements plus `@startNumber` if present; a value above one plus the previously computed number says earlier Segments are unavailable), else continue counting.
3. Repeat count: `@r` (default 0) is zero-based, so the run has `r + 1` Segments (`r = 3` gives four Segments of duration `d`).
4. **Negative `@r`**: the duration `@d` repeats up to the `@t` of the next `S`; for the last `S` it repeats to the end of the Period or the next MPD update, exactly as `@duration` would for a whole
   Period. The spec gives no rounding rule for the resulting count; derive it from the Period-duration rules of 5.3.9.5.3.
5. Advance: `time += d * (r + 1)`, `number += r + 1` (for sequences see below).

Constraints: every `@d` must not exceed `MPD@maxSegmentDuration`. `@d` must be the accurate presentation duration of the Segment, **except** for the single Segment that simultaneously (a) is the `S`
with the largest `@t` in the MPD, (b) belongs to a Representation with `@availabilityTimeOffset` and `@availabilityTimeComplete=false`, and (c) has a non-adjusted availability time at or after the
MPD's publish time.
For that Segment `@d` is nominal and may be corrected in a later MPD (or a new `S` added); with `$Time$` addressing a client should not rely on nominal durations to form addresses, but parse the Segment or refetch the MPD after the non-adjusted availability time.

**`$Time$` addressing with SegmentTemplate** (when `@media` contains `$Time$`):

- If a `sidx` box exists, the timeline must match it exactly: `@timescale` equals the first `sidx` timescale; `S@t` equals the `earliest_presentation_time` of the first `sidx` in the described Media Segment; `S@d` equals the sum of all `subsegment_duration` fields of that first `sidx`.
- If no `sidx`, the earliest presentation time comes from media-internal data, depending on the Segment format.
- The URL is the template with `$Time$` replaced by that earliest presentation time. Because the next Segment's earliest presentation time can be read from the actual Segment, URLs can be built without refetching an updated MPD.

<a id="c5-3-9-6-2"></a>

## 5.3.9.6.2 SegmentTimeline semantics (Table 22, p. 88-89)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `S` | 0..N | - | - | One run of identical-duration Segments; entries ordered by increasing `@t`. |
| `@t` | O | unsigned long | derived as above | Start time of the run's first Segment, minus `PTO` = MPD start time relative to Period start. Must be at least the previous run's end; larger means a timeline gap. |
| `@n` | O | unsigned long | continues counting | Segment number of the run's first Segment (constraint above). |
| `@d` | M | unsigned long | - | Duration of one Segment (or of one Segment sequence, see below). |
| `@k` | OD | unsigned long | `1` | Number of Segments per Segment sequence; only where the profile permits. |
| `@r` | OD | integer | `0` | Zero-based repeat count of following contiguous Segments/sequences of duration `@d`; negative = until next `S@t` / end of Period / next MPD update. |

<a id="c5-3-9-6-3"></a>

## 5.3.9.6.3 Type outline

`SegmentTimelineType` = `S*` then any other-namespace elements; `S` carries `@t`, `@n` (unsignedLong), `@d` (unsignedLong, required), `@r` (`xs:integer`, default 0), `@k` (unsignedLong, default 1) and
other-namespace attributes. Ed.6 changes (`Pattern` child, `S` 1..N, `@d` optional, `@k` unsignedInt, `@p`, `@pE`, `@ssp`): [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

<a id="c5-3-9-6-4"></a>

## 5.3.9.6.4 Segment sequences (p. 90)

`@k` may be present only if (i) the Representation uses a SegmentTemplate with `$Number$` or the hierarchical `$SubNumber$` scheme of 5.3.9.6.5, and (ii) the profile explicitly allows Segment sequences. When `@k > 1`:

- `@d` is the accurate duration of the whole sequence of `@k` Segments. The MPD duration of one Segment is the integer quotient `floor(d / k)`; it drives MPD start time and hence availability, and need not equal the media duration of that Segment.
- Constraint between runs: the integer quotient of `@d` and `@k` of any `S` must not exceed the quotient of `@d` and (`@k` minus 1) of any other `S` (printed p. 90; read verbatim in the private file before implementing a validator).
- The concatenation of all Segments in a sequence has an accurate total duration equal to `@d`. Any Segment in a sequence may carry inband events.

<a id="c5-3-9-6-5"></a>

## 5.3.9.6.5 Hierarchical templating and sub-numbering (p. 90-91)

With a template containing `$SubNumber$` and a timeline using Segment sequences: `$Time$` becomes the earliest presentation time of the whole sequence (same for every Segment of it); `$Number$` becomes the sequence number (each sequence counted as one Segment);
`$SubNumber$` becomes the 1-based index of the Segment inside the sequence. The next sequence starts at current sequence start plus the duration of the concatenated sequence; for ISOBMFF this can be obtained by summing the track runs of the sequence's Segments.

## Transcription notes (spec quirks)

- Table 22 `@k` points to "subclause 5.3.9.6.3" for more detail; Segment sequences are in 5.3.9.6.4 (p. 89).
- 5.3.9.6.1 has a grammar slip in the exception list: "except for the following conditions are met" (p. 87-88). The four conditions are conjunctive.
- 5.3.9.6.5 NOTE misspells "segment sequence" (p. 91).
- The Table 22 `@t` rule says "equal to or greater than the sum of the previous S earliest presentation time and the sum of the contiguous Segment durations", which is the previous run's end time.
- The number of Segments for a negative `@r` is not given a rounding rule in this clause (see step 4).
