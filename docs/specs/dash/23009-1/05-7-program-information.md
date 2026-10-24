---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.7 - Program information (own-words implementer notes)
clauses: 5.7.1-5.7.3
printed_pages: 113-114
pdf_pages: 121-122
verbatim_source: private/specs-verified/dash/23009-1/05-4_to_5-7.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.7 Program information

## What it defines

Descriptive metadata about the programme in an MPD, one `ProgramInformation` element per language.

Placement: **ProgramInformation** is a child of the MPD element, 0..N (Table 3, [`05-3-1-hierarchy.md`](05-3-1-hierarchy.md)).

## Rules

- When there is more than one `ProgramInformation`, each shall have `@lang` and shall describe the presentation adequately in that language.
- Per language the element can hold title, source and copyright text plus a URL for more information.

## Inventory (Table 31)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| ProgramInformation | element | ProgramInformationType | - | container |
| @lang | O | xs:language (RFC 5646 syntax) | unknown | language(s) of this block |
| @moreInformationURL | O | xs:anyURI (absolute) | unknown | page with more information |
| Title | 0..1 | xs:string | - | presentation title |
| Source | 0..1 | xs:string | - | original source, e.g. content provider |
| Copyright | 0..1 | xs:string | - | copyright statement, usually starting with U+00A9 |

```text
ProgramInformationType
  sequence: Title? Source? Copyright? any(##other, lax)*
  @lang xs:language, @moreInformationURL xs:anyURI, @* ##other (lax)
```

Verbatim text: see `private/specs-verified/dash/23009-1/05-4_to_5-7.md` section 5.7 (printed pp. 113-114).
