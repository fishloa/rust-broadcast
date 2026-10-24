---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.9 - DASH metrics descriptor (own-words implementer notes)
clauses: 5.9.1-5.9.4
printed_pages: 133-135
pdf_pages: 141-143
verbatim_source: private/specs-verified/dash/23009-1/05-9_to_5-10.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.9 DASH metrics descriptor

## What it defines

A trigger an MPD author can use to ask clients to measure and report DASH metrics. The standard defines the trigger (`Metrics` element) and the metric keys (annex D, not transcribed) but **no reporting mechanism**; the `Reporting` children name the scheme to use.

Placement: **Metrics** is a child of the MPD element, 0..N (Table 3, [`05-3-1-hierarchy.md`](05-3-1-hierarchy.md)).

## Inventory (Table 37)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| Metrics | element | MetricsType | - | one request for a set of metrics |
| Metrics@metrics | M | xs:string | - | comma-separated list of metric keys (annex D keys) the client should report |
| Range | 0..N | RangeType | whole content | time interval(s) for collection; absent = whole duration |
| Range@starttime | O | xs:duration | start of consumption | for dynamic MPDs: added to `MPD@availabilityStartTime` to get a wall-clock start; for static MPDs: Media Presentation time relative to the first Period's *PeriodStart*. Example: availabilityStartTime 14:30 and collection from 14:45 gives 0:15 |
| Range@duration | O | xs:duration | whole Media Presentation duration | length of the collection interval, in Media Presentation time |
| Reporting | 1..N | DescriptorType | - | requested reporting method/format (descriptor with scheme URI) |

```text
Metrics
  Range (0..N): attributes starttime, duration
  Reporting (1..N)
  attributes: metrics
```

## Reporting rules (5.9.4)

- Clients should collect the metrics named by the element and report them using one of the schemes in the `Reporting` descriptors.
- Elements with unrecognised reporting schemes are ignored; with several `Reporting` elements it is enough to process one recognised scheme.
- No reporting scheme is specified here; external specifications may define formats and delivery and should take care of privacy.

## Quirks

- 5.9.4 speaks of "the Metric element"; the element is `Metrics`.
- The Range element's type is declared as an element with attributes of type `xs:duration` although the clause text says the duration is "in Media Presentation time".

Verbatim text: see `private/specs-verified/dash/23009-1/05-9_to_5-10.md` section 5.9 (printed pp. 133-135).
