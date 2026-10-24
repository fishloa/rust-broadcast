---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.5 - MPD assembly (XLink subset, remote element entities; own-words implementer notes)
clauses: 5.5.1-5.5.3
printed_pages: 108-110
pdf_pages: 116-118
verbatim_source: private/specs-verified/dash/23009-1/05-4_to_5-7.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.5 MPD assembly (XLink)

## What it defines

A restricted subset of W3C XLink *simple links* that lets an MPD element be fetched from a **remote element entity** (an XML fragment, not a valid MPD) and spliced in. It is how
`Period`, `AdaptationSet`, `InitializationSet`, `EventStream` ... carry `@xlink:href` (see the element tables of 5.3.*, 5.10.2).

## Attribute rules (Table 29)

| Attribute | Rule |
|---|---|
| `@xlink:type` | optional; if used it is fixed to `simple` (only simple links exist) |
| `@xlink:href` | an RFC 3986 URI; in DASH exclusively HTTP URLs; cookies (RFC 6265) may be used to personalise resolution but are out of scope; relative URIs are resolved per 5.6.4 ([`05-6-base-url.md`](05-6-base-url.md)) |
| `@xlink:show` | optional; if used fixed to `embed` (the embedding behaviour is defined by 5.5.3, not by XLink) |
| `@xlink:actuate` | `onLoad` or `onRequest` (default). onLoad: dereference right after loading the MPD; the remote entity may not itself contain an `@xlink:href` with onLoad. onRequest: dereference only when the content is needed for playout (e.g. when play position is about to enter the Period, or when an Adaptation Set link might hold wanted Representations) |

## Processing (5.5.3)

Resolution failures:

1. unresolvable reference: invalid; the URI is dropped and all `@xlink:*` attributes are removed from the referencing element;
2. target of the wrong kind: same treatment;
3. target that contains another `@xlink:href` with onLoad: invalid circular reference, same treatment;
4. the special URN `urn:mpeg:dash:resolve-to-zero:2013` as href: at resolution time the referencing element (with all its `@xlink` attributes) is removed ("resolve to zero");
5. relative hrefs follow 5.6.4.

Format of a remote element entity:

- it yields top-level elements all of the same element type as the referencing element; one or several may be returned (several makes it a non-well-formed XML document, which is acceptable);
- the first returned element replaces the referencing element, the others are inserted immediately after it in document order;
- the entity may itself contain an `@xlink:href` with onRequest, resolved later when needed;
- an empty entity: just remove the `@xlink` attributes from the referencing element and leave its other attributes/children untouched;
- a non-empty entity replaces the element as above;
- after dereferencing, all XLink attributes originally present in the MPD are removed;
- every resource named in the remote entity has the availability end time `MPD@availabilityEndTime`;
- remote entities are not valid MPDs and should not be stored with the `.mpd` extension.

An MPD that is updated (5.4) is judged by the update rules after all remote entities have been resolved; the 5.5.1 text cites a first-edition clause number for this ("5.2.3.5") that is not resolved in this edition's numbering (the update rules are in 5.4.1, [`05-4-mpd-updates.md`](05-4-mpd-updates.md)).

## Quirks

- The three numbered lists in 5.5.3 each restart at 1).
- 5.5.3 rule 1 begins "Only a single top-level element type ... may be obtained" and next allows several elements of that type: the singular refers to the element *type*.

Verbatim text: see `private/specs-verified/dash/23009-1/05-4_to_5-7.md` section 5.5 (printed pp. 108-110).
