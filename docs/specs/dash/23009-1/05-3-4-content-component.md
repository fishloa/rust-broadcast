<a id="c5-3-4"></a>

# ISO/IEC 23009-1:2022 clause 5.3.4 - Media content component (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.4, printed pp. 50-51 (PDF 58-59). Verbatim, page-verified text and Table 8:
`private/specs-verified/dash/23009-1/05-3-4-content-component.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-4-1"></a>

## 5.3.4.1 Overview (p. 50)

Every Adaptation Set contains one or more media content components. Their properties go in `ContentComponent` children of the `AdaptationSet`; with only one component the same properties may sit directly on the `AdaptationSet`.

<a id="c5-3-4-2"></a>

## 5.3.4.2 ContentComponent semantics (Table 8, p. 50)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@id` | O | unsigned int | none | Unique within the containing Adaptation Set. Should equal the component identifier inside the Segments: track id for ISOBMFF, PID for MPEG-2 TS. May also address a sub-component of a single elementary stream (for example mixed next-generation-audio components). |
| `@lang` | O | language | - | Same meaning as `AdaptationSet@lang` (Table 5). |
| `@contentType` | O | top-level media type | - | Same as Table 5. |
| `@par` | O | ratio | - | Same as Table 5. |
| `@tag` | O | string | - | Tag of the component usable for selection toward the decoder. |
| `Accessibility`, `Role`, `Rating`, `Viewpoint` | 0..N | descriptor | - | Same as the AdaptationSet elements of those names. |

<a id="c5-3-4-3"></a>

## 5.3.4.3 Element tree

```text
ContentComponent [@id @lang @contentType @par @tag]    (any other-namespace attribute)
 +- Accessibility* +- Role* +- Rating* +- Viewpoint* +- (other-namespace any)
```

`@id` is `xs:unsignedInt`, `@lang` is `xs:language`, `@contentType` is `RFC6838ContentTypeType`, `@par` is `RatioType`, `@tag` is `TagType` (an unrestricted string). Ed.6 treats this type identically ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

## Transcription notes (spec quirks)

- None found in this clause.
