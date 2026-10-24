---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.3.13 - Resynchronization (Resync element; own-words implementer notes)
clauses: 5.3.13.1-5.3.13.3
printed_pages: 103-106
pdf_pages: 111-114
verbatim_source: private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.3.13 Resynchronization

## What it defines

A way to announce that a Segment contains internal **Resynchronization Points** (byte positions where a client can start container-level parsing without
downloading the Segment from its start), together with how densely they occur. Container formats that want to use the feature must define their own Resync Point and marker
(for ISO BMFF see [`06-segment-formats.md`](06-segment-formats.md), Table 51).

Placement: **Resync** is a common element (0..N) of Adaptation Set / Representation / Sub-Representation (Table 14, [`05-3-7-common.md`](05-3-7-common.md); the table's cross-reference names "5.3.12" although the clause is 5.3.13).

## Resync point model

A Resync Point has four properties:

| Property | Meaning |
|---|---|
| Index | byte offset (or index) from the start of the Segment |
| Time | earliest presentation time of any sample reachable by starting at that point |
| Type | access-point type, e.g. a SAP type of ISO/IEC 14496-12 |
| Marker | boolean: the point can be recognised by parsing a specific structure (true) or only by external signalling (false) |

Starting at a Resync Point plus the Initialization Segment (when present) is enough to parse the container; whether the (possibly encrypted) elementary stream is accessible
depends on the point's type.

Two non-exclusive signalling routes: a binary map in a timed-metadata track of the Representation (suits fully available Segments), or the `Resync` element in the MPD (location and timing hints only). The MPD route exists because side-car metadata Segments may be unavailable in live / low-latency cases.

## Normative behaviour for chunk availability

If `Resync` carries `@dImin` and `@dT` (i.e. adjusted values dImin bytes and dT seconds are derivable) and `@availabilityTimeComplete` is false, then:

1. at the Segment's adjusted availability start time the first chunk is available;
2. the (i+1)-th chunk (i = 1..N, N = number of chunks) is available at: adjusted availability start time + i*dT;
3. if `@rangeAccess` on the BaseURL of the Representation is true, available chunks may be fetched with byte ranges; if false the client should not expect range responses.

(Adjusted availability time: see [`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md) and [`05-6-base-url.md`](05-6-base-url.md).)

## Inventory - Resync (Table 28)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| Resync | element | ResyncType | - | present = Resync Points exist in the Segments of this Representation |
| @type | OD | SAPType | 0 | 0: points allow parsing and decryption with the Initialization Segment; >0: points with the type-0 properties exist for Segments with SAP type not larger than the value (SAP types: 4.5.2) |
| @dT | O | xs:unsignedInt | unknown | maximum distance between the Time values of consecutive points of the declared type, in the Representation's `@timescale` units |
| @dImax | O | xs:float | unknown | maximum distance between consecutive Index values, normalised by `@bandwidth` (multiply by `@bandwidth` for octets) |
| @dImin | OD | xs:float | 0 | minimum distance between consecutive Index values, same normalisation |
| @marker | OD | xs:boolean | false | true: every point carries a detectable marker; a Segment format that uses the feature must define its marker (clause 6) |

```text
ResyncType: attributes only
  @type SAPType default 0, @dT xs:unsignedInt, @dImax xs:float, @dImin xs:float default 0,
  @marker xs:boolean default false, @* ##other (lax)
```

## Quirks

- The overview calls dImin / dT "adjusted values in bytes / in seconds" while Table 28 normalises by bandwidth and uses timescale units; the print does not reconcile the two.
- The sentence defining the adjusted values is garbled in the print.
- Table 28's `@type` description contains an indented paragraph introduced by "for all Segment formats defined in this document" that applies to the "greater than 0" case.

Verbatim text: see `private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md` section 5.3.13 (printed pp. 103-106).
