---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex E (normative) - Byte range requests with regular HTTP GET methods, E.1 to E.3"
pdf_pages: "259-261"
printed_pages: "251-253"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-e-byte-range.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex E - Byte-range requests via plain GET (implementer notes)

## Purpose (E.1, p. 251)

Some deployments cannot honour HTTP partial GET (the `Range` header) or return the whole resource anyway. `BaseURL@byteRange`, when present,
declares a template that turns an original URL plus a byte range into a different URL that a *regular* GET can fetch, with the range encoded
in the URL. Clients should use this only after partial GET has failed. A client that can use only this method should request single units
(whole Segments or whole Subsegments).

## Construction rule (E.2, p. 251-252)

1. `BaseURL@byteRange` is a template string. It must contain both `$first$` and `$last$`; the other identifiers are optional.
2. Result of a plain GET on the generated URL (no `Range` header) should equal the result of the original URL with the range header.
3. Substitute identifiers per the table below.
4. If `$query$` is absent from the template and the original URL has a non-empty query (RFC 3986), append `?` followed by that query.
5. Unknown identifiers: result unspecified; the client should ignore the whole containing `ByteRange` element and carry on as if it were absent.
6. Literal text outside identifiers must keep the result a valid HTTP URL (RFC 3986).

| Identifier | Substituted with |
|---|---|
| `$$` | one literal `$` |
| `$base$` | the original URL's `scheme ":" hier-part` (everything before the query) |
| `$query$` | the original URL's query part; if the query is empty, the template separator next to it is dropped: the character just before `$query$` if it is not `?`, otherwise the one just after |
| `$first$` | offset of the first byte of the range (the `first-byte-pos` a partial GET would carry, RFC 7231) |
| `$last$` | offset of the last byte, inclusive (`last-byte-pos`) |

## Worked examples (our own illustrations of the rule)

Original `http://host/a/b.mp4?token=T`, range `10-99`:

| Template | Constructed URL | Reason |
|---|---|---|
| `$base$/range/$first$-$last$` | `http://host/a/b.mp4/range/10-99?token=T` | no `$query$`, query non-empty, so `?token=T` appended |
| `$base$?$query$&range=$first$-$last$` | `http://host/a/b.mp4?token=T&range=10-99` | query substituted in place |

Original `http://host/a/b.mp4` (no query), same template as the second row: `http://host/a/b.mp4?range=10-99`; the `&` after the empty
`$query$` is removed (separator following `$query$` removed since the preceding char is `?`). The spec's E.3 shows exactly these three cases
with its own host and range values (see the private verbatim file).

## Client implementation notes

- Keep partial GET as the default; fall back to byte-range URL only on failure (E.1).
- Build the URL after `BaseURL` resolution; the original URL is the resolved Segment URL.
- Inclusive `last` means length = last - first + 1.
