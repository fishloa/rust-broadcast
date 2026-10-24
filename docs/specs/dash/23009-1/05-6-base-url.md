---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.6 - Base URL processing (own-words implementer notes)
clauses: 5.6.1-5.6.5
printed_pages: 110-113
pdf_pages: 118-121
verbatim_source: private/specs-verified/dash/23009-1/05-4_to_5-7.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.6 Base URL processing

## What it defines

The `BaseURL` element (levels: MPD, Period, AdaptationSet, Representation) and the rules for turning relative URLs found anywhere in the MPD into absolute ones.

## Reference resolution (5.6.4)

- Every URL in the MPD is resolved per RFC 3986 against the `BaseURL` of its own level, or against the level above when the thing being resolved is itself a BaseURL.
- The document "base URI" (RFC 3986 section 5.1) is the level above the MPD level; if only relative URLs exist and no document base URI can be established, the MPD should not be interpreted.
- Levels carrying base URL information: document (above MPD), `MPD.BaseURL` (5.3.1.2), `Period.BaseURL` (5.3.2.2), `AdaptationSet.BaseURL` (5.3.3.2), `Representation.BaseURL` (5.3.5.2).
- A `BaseURL` on the same level as a URL is part of that URL's resolution.
- Informative: resolution follows RFC 3986 section 5.2 and is **not** string concatenation - when the base URL has a path, everything after its last "/" is replaced by the relative reference.

## Alternative base URLs (5.6.5)

- Several `BaseURL` elements on a level mean the same Segments are reachable at several locations (shall be identical content).
- Absent other criteria a client may use the first one; it may use any suitable algorithm to choose among them.
- A `BaseURL` holding an absolute URL overrides any base URL of a higher level.
- The only selection-related attribute is `@serviceLocation` (a grouping hint); this clause prints no `priority` or `weight` attributes (DVB-DASH, ETSI TS 103 285, adds such attributes; not verified from this PDF).

## Inventory (Table 30)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| BaseURL (content) | - | xs:anyURI | - | the base URL string |
| @serviceLocation | O | xs:string | no relation | BaseURLs with equal value are likely served from one network location (e.g. one CDN) |
| @byteRange | O | xs:string | - | allows turning HTTP partial GETs into plain GETs with the range embedded in the URL, built per annex E.2 (not transcribed) |
| @availabilityTimeOffset | O | xs:double | - | offset used to compute the adjusted segment availability time; adds to the value on SegmentBase (semantics: Table 14 in the print; processing 5.3.9.5.3) |
| @availabilityTimeComplete | O | xs:boolean | - | whether Segments are complete at adjusted availability start; should not be given if also on SegmentBase, and if both exist the BaseURL value is ignored |
| @timeShiftBufferDepth | O | xs:duration | - | smallest guaranteed time-shift buffer for resources using this BaseURL; overrides `MPD@timeShiftBufferDepth`; undefined for static MPDs |
| @rangeAccess | OD | xs:boolean | false | true: byte-range requests may target partially available Segments; a client wanting an aggregating response for a non-zero-first-byte request should use the RFC 8673 convention, i.e. signal a last-byte value of 9007199254740991 so the server answers 206 and keeps streaming rather than waiting and answering 200 with a content-length; false: do not expect range responses |

```text
BaseURL (text = URL)
  attributes: serviceLocation, byteRange, availabilityTimeOffset, availabilityTimeComplete,
              timeShiftBufferDepth, rangeAccess
```

## Quirks

- The two availability attributes cite "Table 14" (common attributes) for their semantics, whereas the Segment Base Information table defines them as well.
- Prose says "relative URLs" in the NOTE with a plural typo; no semantic effect.

Verbatim text: see `private/specs-verified/dash/23009-1/05-4_to_5-7.md` section 5.6 (printed pp. 110-113).
