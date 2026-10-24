<a id="c5-3-8"></a>

# ISO/IEC 23009-1:2022 clause 5.3.8 - Subsets (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.8, printed pp. 70-71 (PDF 78-79). Verbatim, page-verified text and Table 15:
`private/specs-verified/dash/23009-1/05-3-8-subsets.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-8-1"></a>

## 5.3.8.1 Overview (p. 70-71)

- A `Subset` (child of `Period`) restricts which Adaptation Sets may be active together. "Active" means the client is presenting at least one Representation of that set.
- Rule the author is expressing: at any moment the set of active Adaptation Sets must be contained in the member list of one of the Period's Subsets.
- An Adaptation Set that is not named in any Subset is implicitly a member of every Subset.
- Consequences: an empty Subset is not allowed, and no Subset should contain all the Adaptation Sets (that would restrict nothing).
- Membership comes from the ids listed in `@contains`.

<a id="c5-3-8-2"></a>

## 5.3.8.2 Subset semantics (Table 15, p. 71)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@contains` | M | list of unsigned ints | - | `AdaptationSet@id` values of the members. |
| `@id` | O | string | none | Identifier of the Subset. |

<a id="c5-3-8-3"></a>

## 5.3.8.3 Type outline

`SubsetType` has no child elements: required `@contains` (`UIntVectorType`), optional `@id` (`xs:string`), any-other-namespace attributes. Identical in Ed.6 ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

## Transcription notes (spec quirks)

- None found in this clause.
