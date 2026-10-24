<a id="c5-3-9-7"></a>

# ISO/IEC 23009-1:2022 clause 5.3.9.7 - Failover content signalling (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.9.7, printed pp. 92-93 (PDF 100-101). Verbatim, page-verified text and Table 23:
`private/specs-verified/dash/23009-1/05-3-9-7-failover-content.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-9-7-1"></a>

## 5.3.9.7.1 Purpose (p. 92)

Sometimes a time-continuous stretch of a Representation or Adaptation Set is not the real content (ingest or encoder error). At DASH level the MPD may still look correct, either through Missing Segments (6.2.6) or by pointing at encoded failover content.
`FailoverContent` marks such stretches so a client can avoid them, for example by moving to a Representation without failover content. It holds a list of `FCS` entries; each has a mandatory `@t` and an optional `@d`; `@t` minus `@presentationTimeOffset` is the MPD start time of the section.

<a id="c5-3-9-7-2"></a>

## 5.3.9.7.2 Semantics (Table 23, p. 92)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `FailoverContent@valid` | OD | boolean | `true` | Whether requests for the affected Media Segments are valid; false means a 404 is expected for them. |
| `FCS` | 1..N | - | - | One continuous failover section. |
| `FCS@t` | M | unsigned long | - | Start of the section's first sample, in timescale units, minus `@presentationTimeOffset` = MPD start time. Must not be earlier than the previous section's end. |
| `FCS@d` | O | unsigned long | until next `FCS`, Period end or MPD duration end, whichever is first | Section duration in timescale units. |

<a id="c5-3-9-7-3"></a>

## 5.3.9.7.3 Type outline

`FailoverContentType` = `FCS+` (attributes `t` unsignedLong required, `d` unsignedLong, other-namespace attributes) then other-namespace elements, with `@valid` (boolean, default true).
`FailoverContent` itself is an optional child of `SegmentBase` (5.3.9.2.2, Table 16). Identical in Ed.6 ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

## Transcription notes (spec quirks)

- None found in this clause; the clause ends at the heading of 5.3.10 (printed p. 93), which is outside the transcribed range.
