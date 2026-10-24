---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.15 - MPD patch framework (own-words implementer notes)
clauses: 5.15.1-5.15.5
printed_pages: 155-159
pdf_pages: 163-167
verbatim_source: private/specs-verified/dash/23009-1/05-11_to_5-15.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.15 MPD patch framework

## What it defines

Instead of re-sending the whole MPD on every update of a dynamic MPD, the server can publish an **MPD Patch (MPP)** document: an XML Patch (RFC 5261: `add`, `remove`, `replace`) that, applied to the in-memory MPD, yields the new MPD. The framework has four parts: the `PatchLocation` element in the MPD, the patch document, the processing model, and recommended client behaviour.

## PatchLocation (5.15.2, Tables 48-49)

Placement: child of the MPD element, 0..N (Table 3, [`05-3-1-hierarchy.md`](05-3-1-hierarchy.md)).

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| PatchLocation (content) | 0..N under MPD | xs:anyURI | - | where a patch that applies to this MPD can be fetched; the target conforms to 5.15.3 |
| @ttl | O | xs:double | unknown | seconds, counted from `MPD@publishTime`, during which the patch document is at least available at that location (the print cites "5.14.4" for the processing model; it is 5.15.4) |

```text
PatchLocationType: simple content, extension of xs:anyURI; @ttl xs:double (optional); @* ##other (lax)
```

## Patch document (5.15.3)

- It is an XML document conforming to the schema of annex B (namespace `urn:mpeg:dash:schema:mpd-patch:2020`, which includes the RFC 5261 patch-ops schema); one `Patch` element; extending the MPP namespace is reserved to ISO/IEC.
- After removing foreign-namespace (DASH-namespace but not in annex B) content it has to remain valid per the schema; likewise after removing everything outside the MPP namespace.
- Operations use XPath 1.0 selectors restricted by RFC 5261 and by 5.15.3.4.
- Encoding UTF-8 (RFC 3629) including extension namespaces; binary data must be Base64 (RFC 4648) in a UTF-8 element with a proper namespace. Transport delivery is out of scope; over HTTP transfer encoding per RFC 7230 is allowed. MIME type: annex C; examples: annex G.21.

Patch element (Table 50):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| Patch | 1 | PatchType | - | root |
| @mpdId | M | xs:string | - | id of the MPD the patch is for |
| @publishTime | M | xs:dateTime | - | when the patch was generated/published at the origin |
| @originalPublishTime | M | xs:dateTime | - | publish time of the MPD the patch is built against; applies only to the MPD with exactly this `MPD@publishTime`, else ignored |
| add, remove, replace | 0..N each (choice, at least one overall) | RFC 5261 types | - | patch operations |
| any element from another namespace | in the choice | - | - | extension |

```text
PatchType: choice (minOccurs=1, unbounded) of { add, remove, replace, any(##other, lax) };
  @mpdId string required, @publishTime dateTime required, @originalPublishTime dateTime required, @* ##other
```

### Element addressing restrictions (5.15.3.4)

- Elements with an `@id` that must be unique among siblings are addressed **only** by that id; in particular `Period`, `AdaptationSet`, `Representation`, `SubRepresentation`.
- Descriptor-type elements (those with `@schemeIdUri`, e.g. `SupplementalProperty`) are addressed by the value of `@schemeIdUri`.
- `S` elements (SegmentTimeline): by their `@t` or `@n` attribute when present, otherwise by position.
- Every other element: by position (e.g. `SegmentTemplate[1]`).
- Examples: `/MPD/Period[@id="1"]/AdaptationSet[@id="1"]/SegmentTemplate[1]/SegmentTimeline[1]/S[@t="12345"]` and `.../S[42]@r` (the printed examples are wrapped mid-word at "Segment Timeline").

## Processing model (5.15.4)

- The MPD derived by patching shall be identical, in W3C Canonical XML form and before any XLink resolution, to the full MPD obtained at `MPD.Location` (if present) or at the URL of the initial MPD with the same `MPD@publishTime`.
- The in-memory MPD need not be stored as XML, but DASH identifiers and the element order of same-typed events must be preserved (addressing modes of the clause).
- Availability: the patch is guaranteed available from `MPD@publishTime` to `MPD@publishTime + PatchLocation@ttl`.
- Patches are applied **before** XLink dereferencing (a patch that creates a new remote Period is followed by dereferencing). Patch application is not expected to affect Periods already resolved through XLink unless the generator knows the resolved Period's `@id`.

## Recommended client operation (5.15.5)

A patch is valid if and only if all of: `Patch@mpdId` equals `MPD@id` of the in-memory MPD; `Patch@originalPublishTime` equals `MPD@publishTime` of the in-memory MPD; `Patch@publishTime` is greater than `MPD@publishTime` of the in-memory MPD; the patch contains a `replace` that sets `MPD@publishTime` to `Patch@publishTime`. The client is expected to check all four. If any fails, patching may produce an MPD different from the real one (fragile, inconsistent behaviour), so the client should fetch the full MPD and ignore the patch; likewise if the request falls after the availability window. If all hold, apply the patch to produce the new MPD.

DASH-event carriage of patches (`urn:mpeg:dash:event:2012` value 2): see [`05-10-events.md`](05-10-events.md).

## Quirks

- 5.15.1 cites "subclause 5.2.3" for where the `PatchLocation` element is specified (the MPD element table is in 5.3.1) and Table 48 cites "5.14.4" for the processing model (it is 5.15.4); Table 49 is an XML listing captioned as a table.
- 5.15.4 refers to "5.15.3.3 ... XPath addressing modes"; the addressing restrictions are in 5.15.3.4.
- Table 50 says "subrclause 4.4" (typo) for RFC 5261's remove.

Verbatim text: see `private/specs-verified/dash/23009-1/05-11_to_5-15.md` section 5.15 (printed pp. 155-159).
