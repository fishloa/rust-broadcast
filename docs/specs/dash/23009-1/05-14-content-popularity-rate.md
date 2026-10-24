---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.14 - Content Popularity Rate (own-words implementer notes)
clauses: 5.14.1-5.14.3
printed_pages: 152-154
pdf_pages: 160-162
verbatim_source: private/specs-verified/dash/23009-1/05-11_to_5-15.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.14 Content Popularity Rate

## What it defines

Per-Preselection / per-Adaptation-Set hints that some content (viewpoints, recommended sub-picture sets of a 360 video, ...) is expected to be consumed more than other content, expressed as a popularity number over runs of Segments. Intended for prefetching by clients or CDNs, for choosing the delivery path (e.g. multicast/broadcast for popular data, unicast for the rest) and for fetching likely-popular data in higher quality. The values come from the author's intent, expected consumption, or measured statistics (e.g. the "Play list" DASH metric of annex D.4.6).

Placement: element **ContentPopularityRate** (0..N) is one of the common elements of Adaptation Set / Representation / Preselection (Table 14, [`05-3-7-common.md`](05-3-7-common.md)).

## Inventory (Table 47)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| ContentPopularityRate | element | ContentPopularityRateType | - | container |
| @source | M | enumeration { content, statistics, other } | - | where the rating comes from: author/provider, viewing statistics, or something else |
| @source_description | O | xs:string | - | UTF-8 text describing the source (should be given when `@source` = other) |
| PR | 1..N | anonymous complex type | - | a run of Segments with identical popularity |
| PR@popularityRate | M | unsignedInt, 1..100 | - | relative popularity within the containing entity inside this Media Presentation; unitless; larger = more likely to be requested |
| PR@start | O | xs:unsignedLong | see below | first Segment the rate applies to: with a `$Number$` Segment template it is the Segment *number* (applies from that Segment's earliest presentation time); for other addressing schemes it is a presentation time in the Representation's `@timescale` units and the rate applies from the Segment that contains the sample at that time |
| PR@r | OD | xs:int | 0 | repeat count: number of Segments after the `@start` Segment that also belong to this run; a negative value means "until the Segment before the first Segment of the next PR, or, with no further PR, until the end of the Period or the next MPD update" |

If `PR@start` is absent the run starts at the first Segment of the Period (first PR element) or at the Segment following the previous PR's last Segment (later PR elements).

```text
ContentPopularityRate
  PR (1..N): attributes popularityRate, start, r
  attributes: source, source_description
```

## Quirks

- Table 47's legend line ends with a full stop and uses a different legend wording than most tables; no semantic effect. The "Pre-selection" spelling appears once in 5.14.1.
- The clause does not say how a client should treat overlapping or gap-leaving PR runs.

Verbatim text: see `private/specs-verified/dash/23009-1/05-11_to_5-15.md` section 5.14 (printed pp. 152-154).
