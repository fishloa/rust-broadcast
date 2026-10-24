---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.12 - Producer Reference Time (own-words implementer notes)
clauses: 5.12.1-5.12.3
printed_pages: 148-150
pdf_pages: 156-158
verbatim_source: private/specs-verified/dash/23009-1/05-11_to_5-15.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.12 Producer Reference Time

## What it defines

A correlation between media timestamps and wall-clock production time, usable by clients to run consumption at a rate matching production (avoiding buffer under/overflow) and to measure/control end-to-end latency. It follows the ISO/IEC 14496-12 `prft` box and can be carried inband as `prft` in Segments, in the MPD as the **ProducerReferenceTime** element, or both.

## Inventory (Table 45)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| ProducerReferenceTime | element | ProducerReferenceTimeType | - | for the associated Representations |
| @id | M | xs:unsignedInt | - | identifier of this Producer Reference Time in the MPD |
| @inband | OD | xs:boolean | false | true: every Segment or Subsegment shall contain a `prft` (ISO/IEC 14496-12) whose flags match `@type` |
| @type | OD | ProducerReferenceTimeTypeType | encoder | semantics of the reference (below) |
| @applicationScheme | CM | xs:string | - | application scheme the time conforms to; should be present when type = application, shall not be present otherwise |
| @wallClockTime | M | xs:string | - | wall-clock time: without a `UTCTiming` child it is the NTP format of the `prft` `ntp_timestamp` field; with a `UTCTiming` child it has the format of that UTC timing scheme (convert if extracted from a `prft` box) |
| @presentationTime | M | xs:unsignedLong | - | presentation time in the Representation's timescale that corresponds to `@wallClockTime` (derived from `media_time` if taken from a `prft` box) |
| UTCTiming | 0..1 | DescriptorType | - | when present the wall-clock times are synchronised to this timing anchor, and the same UTC Timing descriptor shall also be in the MPD ([`05-8-5-schemes.md`](05-8-5-schemes.md), 5.8.5.7) |

`@type` values (`ProducerReferenceTimeTypeType` = string enumeration { encoder, captured, application }) and the matching `prft` flags of ISO/IEC 14496-12:

| Value | Reference point | prft flags |
|---|---|---|
| encoder | media time when the sample entered the encoder | flags = 0 |
| captured | media time when the sample was captured | flags 8 and 16 both set |
| application | wall-clock relation defined by an application scheme | flag 16 set, flag 8 unset |

```text
ProducerReferenceTimeType: sequence UTCTiming? (DescriptorType), any(##other)*;
  @id unsignedInt required, @inband boolean default false, @type ProducerReferenceTimeTypeType default encoder,
  @applicationScheme string, @wallClockTime string required, @presentationTime unsignedLong required, @* ##other
```

## Quirks

- The `ProducerReferenceTimeType` complex type is printed in the XML listing of 5.3.10.3 (printed p. 94), not in 5.12.3 (which only lists the `ProducerReferenceTimeTypeType` enumeration, printed p. 150).
- Table 45 `@id` description reads "Producer Reference ime" (missing T). The NTP clause says "NTP format associated to @media" (an unexplained reference to a `@media` attribute).
- Placement is not stated in this clause: per Table 14 (see [`05-3-7-common.md`](05-3-7-common.md)) it is a common element, 0..N, of Adaptation Set, Representation, SubRepresentation and Preselection.

Verbatim text: see `private/specs-verified/dash/23009-1/05-11_to_5-15.md` section 5.12 (printed pp. 148-150).
