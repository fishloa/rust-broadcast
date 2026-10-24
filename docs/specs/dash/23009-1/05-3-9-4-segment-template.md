<a id="c5-3-9-4"></a>

# ISO/IEC 23009-1:2022 clause 5.3.9.4 - Segment template (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.9.4, printed pp. 82-83 (PDF 90-91). Verbatim, page-verified text, Tables 20-21 and XML syntax:
`private/specs-verified/dash/23009-1/05-3-9-4-segment-template.md`. Companion notes: where the number and time values come from is in
[`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md) (Number) and [`05-3-9-6-segment-timeline.md`](05-3-9-6-segment-timeline.md) (Time). See [`INDEX.md`](INDEX.md).

<a id="c5-3-9-4-1"></a>

## 5.3.9.4.1 Overview (p. 82)

`SegmentTemplate` produces a list of Segment URLs by substituting dynamic values into URL patterns; the substitution rules are in 5.3.9.4.4.

<a id="c5-3-9-4-2"></a>

## 5.3.9.4.2 Semantics (Table 20, p. 82)

`SegmentTemplate` extends MultipleSegmentBaseInformation ([`05-3-9-segment-information.md`](05-3-9-segment-information.md#c5-3-9-2-2), Table 17) and adds four optional attributes, all strings holding URL templates:

| Attribute | Use | Rule |
|---|---|---|
| `@media` | O | Template for the Media Segment list. |
| `@index` | O | Template for the Index Segments; if it contains neither `$Number$` nor `$Time$` it is the URL of a single Representation Index. |
| `@initialization` | O | Template for the Initialization Segment; must contain neither `$Number$` nor `$Time$`. |
| `@bitstreamSwitching` | O | Template for the Bitstream Switching Segment; must contain neither `$Number$` nor `$Time$`. |

<a id="c5-3-9-4-3"></a>

## 5.3.9.4.3 Type outline

`SegmentTemplateType` extends `MultipleSegmentBaseType` with four `xs:string` attributes: `media`, `index`, `initialization`, `bitstreamSwitching`. Identical in Ed.6 (the inherited base differs, see [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

<a id="c5-3-9-4-4"></a>

## 5.3.9.4.4 URL construction algorithm (p. 82-83)

Applies to each of the four template attributes. Behaviour, restated:

1. **Scan** the string for `$`-delimited identifiers (Table 21 below). Matching is case-sensitive. `$$` is an escape for one literal `$`.
2. **Invalid template**: a `$` that does not enclose a valid identifier makes URL formation undefined; the client is expected to drop the entire containing Representation and carry on as if it were absent.
3. **Format tag**: inside the delimiters an identifier may be followed by a printf-style tag of the shape `%0<width>d` (IEEE 1003.1-2008). `width` is an unsigned integer giving the minimum character
   count; shorter values are left-padded with zeros; longer values are never truncated. `$RepresentationID$` and `$$` take no tag; for the others, with no tag present the tag `%01d` (width 1) applies.
4. **Authoring constraint**: the resulting URLs must be valid, and text outside identifiers may only contain RFC 3986 URL characters.

Identifier table (Table 21, our wording):

| Identifier | Replaced by | Format tag |
|---|---|---|
| `$$` | one `$` | n/a |
| `$RepresentationID$` | `Representation@id` of the containing Representation | not allowed |
| `$Number$` | number of the Segment; if `$SubNumber$` also occurs in the same string, the number of the Segment *sequence* instead (see 5.3.9.6.4/5.3.9.6.5) | optional, default width 1 |
| `$Bandwidth$` | `Representation@bandwidth` | optional, default width 1 |
| `$Time$` | the MPD start time of the Segment; with a SegmentTimeline that is the `@t` value of the entry for the requested Segment. `$Number$` and `$Time$` must not both be used | optional, default width 1 |
| `$SubNumber$` | number of the Segment within a Segment sequence; may appear only together with `$Number$` or `$Time$` (5.3.9.6.4/5.3.9.6.5) | optional, default width 1 |

Worked reading: template `seg-$RepresentationID$-$Number%05d$.m4s` for Representation `v1` and Segment number 7 yields `seg-v1-00007.m4s`; `$Time$` behaves the same with the time value in timescale units.

## Transcription notes (spec quirks)

- 5.3.9.4.4 first paragraph after the identifier rules says the identifiers "from Table 20" are replaced by the substitution parameter "defined in Table 16" (p. 82). Both table numbers are wrong: the
  identifiers are in Table 21, and Table 16 is the SegmentBase semantics table. Table 21 is the operative table.
- The `$SubNumber$` row misspells "Segment Sequence" as "Seqment Sequence" (p. 83).
