<a id="c5-3-6"></a>

# ISO/IEC 23009-1:2022 clause 5.3.6 - Sub-Representation (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.6, printed pp. 62-63 (PDF 70-71). Verbatim, page-verified text and Table 13:
`private/specs-verified/dash/23009-1/05-3-6-sub-representation.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-6-1"></a>

## 5.3.6.1 Overview (p. 62)

- `SubRepresentation` children of a `Representation` describe one or more content components embedded in it: for example an embedded audio track (codec, sampling rate), a subtitle stream, or a lower-quality video layer (lower frame rate). They share the common attribute group (5.3.7).
- With `@level` present: the Sub-Representation is a way to reach a lower-quality version of its Representation (for example pull the audio out of a multiplex, or a low-frame-rate layer for
  fast-forward/rewind). The Initialization, Media and/or Index Segments must then make the data reachable by HTTP partial GET; for the media formats defined in this document that means the Subsegment
  Index of 6.3.2.4 must be used.
- With `@level` absent: the element merely adds detail about embedded streams.

<a id="c5-3-6-2"></a>

## 5.3.6.2 SubRepresentation semantics (Table 13, p. 62-63)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@level` | O | unsigned int | none | The level number. If present, a Subsegment Index (6.3.2.4) must be available for each Media Segment of the Representation (for formats in this document). Absent: description-only element. |
| `@dependencyLevel` | O | list of unsigned ints | none (independent) | `@level` values of the Sub-Representations of the same Representation this one depends on for decode/presentation. |
| `@bandwidth` | CM (required when `@level` is present) | unsigned int | - | Same meaning as `Representation@bandwidth`, applied to this Sub-Representation. |
| `@contentComponent` | O | list of ids | none | `ContentComponent@id`s of all components in this Sub-Representation; absent means not assigned to a component. |
| CommonAttributesElements | - | - | - | `RepresentationBaseType`, 5.3.7. |

<a id="c5-3-6-3"></a>

## 5.3.6.3 Type outline

`SubRepresentationType` extends `RepresentationBaseType` with attributes `@level` (`xs:unsignedInt`), `@dependencyLevel` (`UIntVectorType`), `@bandwidth` (`xs:unsignedInt`), `@contentComponent`
(`StringVectorType`) and no extra child elements. Identical in Ed.6 ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

## Transcription notes (spec quirks)

- 5.3.6.1 prints "6.3.2.4shall" without a space (p. 62).
