---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.3.11 - Preselection (own-words implementer notes)
clauses: 5.3.11.1-5.3.11.5.3
printed_pages: 95-99
pdf_pages: 103-107
verbatim_source: private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.3.11 Preselection

## What it defines

A **Preselection** is a named, selectable "complete experience" assembled from one or several Adaptation Sets of the same media type (first built for
next-generation audio, but generic). It exists in two forms: a descriptor on an Adaptation Set (simple, backward compatible) and a dedicated **Preselection**
element (full feature set).

## Concepts

- Main Adaptation Set: the one whose Initialization Segment initialises the whole experience; one Representation of it is always needed. A Preselection names exactly one main set (the first id in its list) and zero or more further sets.
- Partial Adaptation Set: only usable together with the main set of that Preselection.
- "Main" here is unrelated to the `main` Role value.
- Components can be referenced by `AdaptationSet@id` (single component, or a single multiplexed-at-elementary-stream-level track that carries predefined experiences selected by passing a tag to the media engine) or by `ContentComponent@id` (several tracks in one Adaptation Set, container-level muxing). If Content Components are referenced, Adaptation Set ids and Content Component ids must be unique within the Period.
- A Preselection can carry a tag; the mapping of tags to codec features is left to the codec's own documentation.

## Preselection descriptor (5.3.11.2)

| Aspect | Rule |
|---|---|
| Carrier | an EssentialProperty or SupplementalProperty with `@schemeIdUri` = `urn:mpeg:dash:preselection:2016` (the print spells the attribute `@schemeIdURI` here) |
| Placement | Adaptation Set level only |
| Value | optional; when present: `tag "," id-list`, the id list is whitespace separated, first id = main Adaptation Set |
| Value grammar | two fields separated by a comma: a free-text tag, then one or more decimal ids separated by white space (the print's grammar names are `PRESELECTION-DESCRIPTOR-VALUE`, `ID-LIST`, `TAG-VALUE`, `ID-VALUE`) |
| Descriptor without a value | the Adaptation Set shall be referenced by at least one Preselection |
| Descriptor with a value | it defines a Preselection (tag + member ids) |
| Essential flavour | the Adaptation Set is only consumable as part of a Preselection |
| Supplemental flavour | the Adaptation Set is also consumable on its own |

## Preselection element (5.3.11.3, Table 26)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| Preselection@id | OD | StringNoWhitespaceType | "1" | identifier, unique inside one Period |
| @preselectionComponents | M | StringVectorType | - | whitespace-separated ids (Adaptation Sets or Content Components) in processing order; first is the main set |
| @lang | O | xs:language | - | as AdaptationSet@lang |
| @order | OD | PreselectionOrderType | undefined | which conformance rule applies |
| Accessibility, Role, Rating, Viewpoint | 0..N each | DescriptorType | - | annotation descriptors as in 5.8 |
| common RepresentationBaseType attributes/elements | - | - | - | the shared set of 5.3.7 |

Enumeration `@order`: `undefined`, `time-ordered`, `fully-ordered`.

```text
Preselection (RepresentationBaseType content plus)
  Accessibility, Role, Rating, Viewpoint (0..N each)
  attributes: id, preselectionComponents, lang, order   (order values: undefined, time-ordered, fully-ordered)
```

## Conformance rules (5.3.11.5)

| @order | Rule |
|---|---|
| undefined | each Adaptation Set follows the ordinary multi-segment-track rules (see [`05-3-5-representation.md`](05-3-5-representation.md), 5.3.5.1); no extra rules between sets |
| time-ordered | the main set's Initialization Segment followed by media (sub)segments of one Representation per member set, sorted by non-decreasing first decode time, must form a conforming Segment track (4.5.4) with the main Representation's `@mimeType`; ties are unordered; member Representations *should* be segment/subsegment aligned (5.3.3.5) |
| fully-ordered | same concatenation, but sorted by non-decreasing decode time and then by position in `@preselectionComponents`; alignment is *shall* |

## Quirks

- 5.3.11.2 says the "Multi-Segment Track" rule of 5.3.11.5.2 applies to descriptor-defined Preselections, but 5.3.11.5.2 is the time-ordered rule (multi-segment is 5.3.11.5.1).
- The table caption says "PreSelection"; the printed grammar for the id list admits at most two ids although the prose describes a list of any length, and it leaves white space undefined.
- The time-ordered rule's first list item has an unbalanced parenthesis.
- Table 26's `@id` default is printed "default=1", `@order` "Default: 'undefined'".

Verbatim text: see `private/specs-verified/dash/23009-1/05-3-10_to_5-3-13.md` section 5.3.11 (printed pp. 95-99).
