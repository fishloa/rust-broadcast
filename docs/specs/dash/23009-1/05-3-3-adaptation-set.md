<a id="c5-3-3"></a>

# ISO/IEC 23009-1:2022 clause 5.3.3 - Adaptation Sets (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.3, printed pp. 39-50 (PDF 47-58). Verbatim, page-verified text, Tables 5-7 and the XML syntax:
`private/specs-verified/dash/23009-1/05-3-3-adaptation-set.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-3-1"></a>

## 5.3.3.1 Overview and grouping rules (p. 39-41)

- An Adaptation Set (AS) holds alternate Representations: at most one is presented at a time. They carry the same content components and are perceptually equivalent. AS and Representations must be
  prepared so that seamless switching (4.5.1) across the Representations is possible; if consumers are expected to have switching limits, the author should offer enough means to switch seamlessly
  within them.
- **Membership rule (if and only if).** Representations share an AS exactly when, for each content component, they have identical values of: language (`@lang`), content type (`@contentType`), picture aspect ratio (`@par`), Role, Accessibility, Viewpoint and Rating.
  Values of the four descriptor elements are mostly defined outside this document, except the simple schemes of 5.8.5.
- Several content components: each is described by its own `ContentComponent` (5.3.4). Defaults or values that apply to all components go on the `AdaptationSet`; an attribute present on the AdaptationSet must not be repeated on a ContentComponent.
- The common attribute/element group (5.3.7) may give defaults for contained Representations, but each common attribute may appear on the AS or on the Representation, never both.
- Range attributes (`@minBandwidth`..`@maxFrameRate`) summarise the contained Representations; no Representation may fall outside the declared range.
- **Group rule (`@group`).** Within one Period the content is either exactly one Representation from group 0 (if group 0 is used), or the combination of at most one Representation from each non-zero group. A set without `@group` forms its own private non-zero group.

<a id="c5-3-3-2"></a>

## 5.3.3.2 AdaptationSet semantics (Table 5, p. 40-45)

Without xlink the Use codes hold; with `@xlink:href` all attributes become optional and elements minOccurs 0. Fixed `F` is also a code in this table's key.

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@xlink:href` | O | URI | - | Remote entity that must contain exactly one AdaptationSet. |
| `@xlink:actuate` | OD | `onLoad`/`onRequest` | `onRequest` | Resolution timing. |
| `@id` | O | unsigned int | none | Unique within the containing Period; must not be present in a remote element entity. (Analysis: [`dash-23009-1-2022-ids.md`](dash-23009-1-2022-ids.md).) |
| `@group` | O | unsigned int | see 5.3.3.1 | Group identifier, unique within the Period. |
| CommonAttributesElements | - | - | - | The `RepresentationBaseType` group, 5.3.7. |
| `@lang` | O | RFC 5646 language tag | unknown | Language of the set. `und` or `zxx` may be used for undetermined or non-linguistic. If absent, may be set per component or be unknown. |
| `@contentType` | O | top-level media type (RFC 6838) | unknown | Component type: `text`, `image`, `audio`, `video`, `application`, `font`. |
| `@par` | O | ratio `w:h` | unknown | Picture aspect ratio of the video. |
| (cont.) |||| When `@par` and the Representations' `@width`/`@height` are all present it must express the same ratio as (`@width` * sarx) : (`@height` * sary), with sarx:sary the two numbers of `@sar`. |
| (cont.) |||| (Analysis: [`dash-23009-1-2022-aspect.md`](dash-23009-1-2022-aspect.md).) |
| `@minBandwidth`, `@maxBandwidth` | O | unsigned int | unknown | Smallest and largest `@bandwidth` of the set (same unit). |
| `@minWidth`, `@maxWidth`, `@minHeight`, `@maxHeight` | O | unsigned int | unknown | Smallest and largest `@width`/`@height` (same unit). |
| `@minFrameRate`, `@maxFrameRate` | O | frame rate | unknown | Extreme `@frameRate`, same encoding. |
| `@segmentAlignment` | OD | boolean | `false` | True: for any Representations X, Y of the set, Segment m of X and Segment n of Y are non-overlapping (4.5.3) whenever m differs from n. For multi-component sets only true/false allowed. Non-boolean values of earlier editions are deprecated. |
| `@bitstreamSwitching` | O | boolean | none | True: all Representations have the same number M of Media Segments, and any sequence built from the set's optional Initialization Segment followed by, for each k = 1..M, Segment k of some Representation i(k) |
| (cont.) |||| (with that Representation's Bitstream Switching Segment first, when such Segments exist) is a conforming Segment track (4.5.4) in the `@mimeType` format. |
| (cont.) |||| Meaning in practice: seamless switching without re-initialising the decoder; use only when decoders need no re-init. |
| `@subsegmentAlignment` | OD | boolean | `false` | True requires every Media Segment to be indexed (own Segment Index or an Index Segment) and Subsegment m of X / Subsegment n of Y to be non-overlapping whenever m differs from n. Non-boolean values deprecated. |
| `@subsegmentStartsWithSAP` | OD | SAP type 0..6 | `0` | When greater than 0: every Subsegment with a SAP type greater than 0 starts with a SAP whose type is at most this value; |
| (cont.) |||| "starts with SAP" means the first SAP in the Subsegment has I_SAU at the first access unit following I_SAP and I_SAP inside the Subsegment. Value 0 is unspecified. |
| `@initializationSetRef` | O | uint list | none | Initialization Sets (5.3.12) that the AS conforms to - all of them. |
| `@initializationPrincipal` | O | anyURI | an Initialization Segment of any Representation | URL of an Initialization Segment sufficient to initialise the whole AS. |
| `Accessibility`, `Role`, `Rating`, `Viewpoint` | 0..N | descriptor | - | Annotation schemes (5.8.1; 5.8.4.3, .2, .4, .5). |
| `ContentComponent` | 0..N | - | - | One per component in a multi-component AS (5.3.4). |
| `BaseURL` | 0..N | - | - | 5.6. |
| `SegmentBase`, `SegmentList`, `SegmentTemplate` | 0..1 each | - | - | AS-level defaults, overridden by the Representation's own (5.3.9). |
| `Representation` | 0..N | - | - | At least one is required (it may live in the xlink target when the AS is remote). |

<a id="c5-3-3-3"></a>

## 5.3.3.3 Element tree and simple types

```text
AdaptationSet : RepresentationBaseType + { xlink attrs, @id @group @lang @contentType @par
                @min/maxBandwidth @min/maxWidth @min/maxHeight @min/maxFrameRate
                @segmentAlignment @subsegmentAlignment @subsegmentStartsWithSAP @bitstreamSwitching
                @initializationSetRef @initializationPrincipal }
 +- Accessibility* +- Role* +- Rating* +- Viewpoint*     (descriptors)
 +- ContentComponent* +- BaseURL* +- SegmentBase? +- SegmentList? +- SegmentTemplate?
 +- Representation*
```

Simple types: `RatioType` = string matching `[0-9]*:[0-9]*`; `FrameRateType` = string matching `[0-9]+(/[1-9][0-9]*)?` (integer or integer/non-zero-integer); `RFC6838ContentTypeType` = enumeration `text|image|audio|video|application|font`;
`SAPType` = unsigned int from 0 to 6 inclusive. `@id`, `@group` and the min/max bandwidth/width/height attributes are `xs:unsignedInt`, `@lang` is `xs:language`. Ed.6 differences: [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

<a id="c5-3-3-4"></a>

## 5.3.3.4 Switching inside an Adaptation Set (Tables 6-7, p. 47-48)

The `Switching` element (instead of, or alongside, `@segmentAlignment`/`@bitstreamSwitching`) announces switch-to points and the permitted strategy.

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `Switching@interval` | M | unsigned int | - | Spacing of switch points in `@timescale` units of the Representation. |
| (cont.) |||| A Segment is a switch-to opportunity when (its earliest presentation time minus the `@t` of the `S` element describing it) is an integer multiple of `interval` (the spec phrases the multiple as being of the product of timescale and interval - see spec note below). |
| (cont.) |||| Should be chosen to land on Segment MPD start times, else nothing is described. |
| `Switching@type` | OD | `media` / `bitstream` | `media` | Strategy at those points. |

Strategies (Table 7): `media` = play the old Representation to t, initialise the new one from its own Initialization Segment, continue from t. `bitstream` = switch by simply continuing with the new
Representation's Segments; the concatenation across the switch is a conforming Segment track in the `@mimeType` format; re-initialisation is neither needed nor recommended, and a shared Initialization
Segment (highest profile/level) is recommended.

Schema: `SwitchingType` has `@interval` (unsignedInt, required), `@type` (`SwitchingTypeType`, default `media`), any other-namespace attribute; `SwitchingTypeType` enumerates `media` and `bitstream`.

<a id="c5-3-3-5"></a>

## 5.3.3.5 Switching across Adaptation Sets (p. 48-49)

- Signal with a SupplementalProperty on an AS: scheme `urn:mpeg:dash:adaptation-set-switching:2016`, `@value` = comma-separated list of AS ids that may be switched to seamlessly from this one. Typical use: same content in AVC and HEVC. Example: AVC set id 4 lists "5", HEVC set id 5 lists "4".
- If alignment flags are true they hold across all Representations of all listed sets; when two sets signal switching, for every pair of Representations X, Y in the union, Subsegment m of X and Subsegment n of Y are non-overlapping for m not equal to n.
- If a `Switching` element is present, its switch points apply across all Representations of all listed sets.
- Parameters that hold for an AS must also hold for every AS named in `@value`; so signalling may legitimately appear on only one of two sets (a set offering all resolutions of another may list it without the converse being true).

## Transcription notes (spec quirks)

- Table 5, `@bitstreamSwitching` ends with "For more details, refer to subclause 0": a broken cross-reference field in print (p. 43). The intended target is not guessed here.
- Table 6 `@interval` says the multiple is "of the product of `@timescale` and `@interval`" although `@interval` is already stated in `@timescale` units (p. 47); implementers should check the Annex B/DASH-IF behaviour before relying on the product reading.
- Table 5 nested list structure in the `@bitstreamSwitching` cell is only legible in the private verbatim file.
