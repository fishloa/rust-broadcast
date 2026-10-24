---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex I (normative) - Flexible Insertion of URL Parameters, I.1 to I.3"
pdf_pages: "301-314"
printed_pages: "293-306"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-i-urlparam.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex I - Flexible insertion of URL parameters (implementer notes)

## I.1 Overview (p. 293)

The annex adds a configurable way to put name/value parameters on requests. Two stages: a parameter is *instantiated* (client builds a name/value
pair) then *output* (written to the query string or to a header).

Instantiation sources: (1) inherit from the query of the URL the MPD was fetched with; (2) just-in-time server computation through XLink;
(3) from a named HTTP response header; (4) client-side computation for a scheme identified by a URN (the client must understand that scheme).
Output modes: URL query parameters, or one DASH-specific HTTP header.

Signalling uses `EssentialProperty` or `SupplementalProperty` descriptors:

| Scheme URN | Scope | Child element | Clause |
|---|---|---|---|
| `urn:mpeg:dash:urlparam:2014` | media Segment requests only | `up:UrlQueryInfo` | I.2 |
| `urn:mpeg:dash:urlparam:2016` | Segment, MPD, XLink, callback (and chaining/fallback) requests; superset of 2014 | `ExtUrlQueryInfo`, `ExtHttpHeaderInfo` | I.3 |

Namespaces and files: element namespace `urn:mpeg:dash:schema:urlparam:2014` (prefix `up:` recommended). The normative schema is
`DASH-MPD-UP.xsd` (external URL in the Ed.5 text; a copy for Ed.6 is in `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/`). If prose and
schema disagree, the schema wins (I.2.1). Not duplicated here.

## I.2 Segment URL parametrization (baseline, 2014)

### I.2.1 Placement rules (p. 294)

- The descriptor must contain a `up:UrlQueryInfo` child.
- May appear at MPD, Adaptation Set or Representation level; at Period level only as `SupplementalProperty`. To force parameters for a whole
  Period, put `EssentialProperty` descriptors on all its Adaptation Sets. At most one such descriptor per level.
- If the parameter values need scheme-specific computation, add further `EssentialProperty`/`SupplementalProperty` descriptors at the same
  level whose `@schemeIdUri` names that scheme; the schemes themselves are out of scope.

### I.2.2 UrlQueryInfo (Table I.1, pp. 294-296)

The query string used for Segment URLs comes from the MPD URL's query (when `@useMPDUrlQuery` is set), from `@queryString`, or from
`@queryString` after XLink resolution. `@queryTemplate` selects which of those parameters appear, and in what order.

| Element / attribute | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `UrlQueryInfo` | - | element | - | carries the query information |
| `@queryTemplate` | O | string | - | template made of `$...$` identifiers (Table I.2); unknown identifiers become empty; an unmatched `$` makes the result undefined and the client must act as if it did not understand the property's scheme; with `$query:param$` the source string must be `name=value` pairs separated by `&` |
| `@useMPDUrlQuery` | O | boolean | false | take parameters from the MPD URL's query; only meaningful if the MPD was fetched over HTTP; if `@queryString` also exists, the MPD query comes first then `@queryString`; across the hierarchy at most one `UrlQueryInfo` may set it true |
| `@queryString` | O | string | - | literal query text; may include scheme placeholders such as `a=$urn:XYZ$`, requiring the client to know that scheme |
| `@xlink:href` | O | URI | - | reference to a remote `UrlQueryInfo` |
| `@xlink:actuate` | OD | `onLoad` or `onRequest` | `onRequest` | only allowed with `@xlink:href` |
| `xlink:type` / `xlink:show` | fixed | - | `simple` / `embed` | from the XML syntax (I.2.2.3) |
| any other-namespace element/attribute | - | - | - | allowed (lax) |

Template identifiers (Table I.2):

| Identifier | Replaced with |
|---|---|
| `$$` | one `$` |
| `$querypart$` | the whole computed query string (`initialQueryString`), the part after `?` up to `#`; at most once and not combined with other identifiers |
| `$query:<param>$` | the value of `<param>` in `initialQueryString`; last occurrence wins; empty if absent |

### I.2.3 URL construction procedure (pp. 297-298)

1. If the `UrlQueryInfo` is remote (`@xlink:href`), dereference it (clause 5.5).
2. Build `initialQueryString`: concatenate (MPD-URL query if `@useMPDUrlQuery`, then `@queryString`), inserting `&` before each later piece.
3. Build `finalQueryString` by substituting `@queryTemplate` identifiers from the table below.
4. Append `finalQueryString` to each Media Segment URL: insert `?` if the Segment URL has no query, `&` if it already has one.
5. If Annex E byte-range construction is also in play, do the URL-parameter step **first**, then the Annex E step.

When several descriptors apply, a Representation's `finalQueryString` is the concatenation, in this order: Representation level, Adaptation
Set level, Period level, MPD level (the MPD-URL-derived query joins first as the source for its own level, per the text; see the verbatim
file for the exact wording).

A straightforward algorithm:
1. Split the concatenated `initialQueryString` on `&` into `name=value` entries; a repeated name overwrites (last wins). If a name is a URN,
   its value is computed by the client (otherwise taken from the string).
2. In `@queryTemplate`, for each `$query:` occurrence read characters up to the next `$` as the parameter name and replace the whole
   identifier by that parameter's value (example idea: template `p0=$query:param0$` with `param0=43` gives `p0=43`).

### I.2.4 Worked behaviour (our own illustrations)

| Case | MPD URL | Descriptor | Resulting Segment URL query |
|---|---|---|---|
| inherit everything | `http://h/x.mpd?token=1&ip=2` | `queryTemplate="$querypart$" useMPDUrlQuery="true"` | `?token=1&ip=2` |
| pick one parameter | same | `queryTemplate="token=$query:token$" useMPDUrlQuery="true"` | `?token=1` |
| server-computed | `http://h/x.mpd` | descriptor with `xlink:href` returning `queryString="param=abc"`, `queryTemplate="$querypart$"` | `?param=abc` |
| client feedback | `http://h/x.mpd?pd=$urn:example:gps$` | extra descriptor with that URN | `?pd=<client value>` (client must know the scheme) |

The spec's four examples (I.2.4.1 to I.2.4.4, pp. 298-302, MPD URLs `urlparam1.mpd` to `urlparam4.mpd`) cover exactly these four cases; they are
listed in [annex-g-examples.md](annex-g-examples.md#annex-i-examples) as an outline and not copied.

## I.3 Extended parametrization (2016)

### I.3.1-I.3.3 Elements (Table I.3, pp. 302-305)

Two elements, both of type `ExtendedUrlInfoType` (an extension of `UrlQueryInfoType`):
- `ExtUrlQueryInfo`: output goes to the URL query.
- `ExtHttpHeaderInfo`: output goes to a custom request header.

Extra attributes on top of Table I.1:

| Attribute | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@includeInRequests` | OD | space-separated list | `segment` | request kinds that carry parameters: `segment`, `xlink`, `mpd`, `callback`, `chaining`, `fallback` |
| `@headerParamSource` | OD | space-separated list | empty (no header inspection); note the table cell reads default "segment" but the text says empty, as printed | which response kinds to read header values from: `segment`, `xlink`, `mpd`, `callback`. If set: `@queryTemplate` must exist and contain `$header:<name>$`, and `@useMPDUrlQuery`/`@queryString` must be absent |
| `@sameOriginOnly` | OD | boolean | false | send parameters only to the origin they came from (RFC 6454 scheme/host/port); for header sources the origin of the request named by `@headerParamSource`; for MPD-derived values the MPD URL's origin |

> **Spec note:** Table I.3 shows `(default: "segment")` for `@headerParamSource` but the description says the default value is the empty
> string. The XML syntax in I.3.3 gives `default="segment"` on both `includeInRequests` and `headerParamSource`. Three printed statements
> disagree; record all three, prefer the XSD (`DASH-MPD-UP.xsd`) when implementing.

Additional identifier (Table I.4): `$header:<header-name>$` -> the latest received value of that HTTP header in the responses selected by `@headerParamSource`.

### I.3.4 Parameter generation (pp. 305-306)

Request kinds: MPD request (for example to `MPD.Location`), XLink dereference, Segment request, callback request (from a DASH callback event,
inband or MPD). Steps: dereference if remote; build `initialQueryString`; build `finalQueryString`; output (I.3.5).

`initialQueryString` is built by exactly one of: (1) the MPD URL's query (when `@useMPDUrlQuery`), followed by `@queryString` if present;
(2) if `@headerParamSource` is non-empty, `<header-name>=<value>` pairs for the headers named in the template, taken from the response types
listed; (3) otherwise `@queryString`. Pieces are joined with `&`.

`finalQueryString` is the template substitution as in I.2.3, plus `$header:...$` replacement from the parameter table. With several
`ExtendedUrlInfoType` elements, each level's result is the concatenation of the current level and all upper levels. Example of the rule: if
both MPD and Period carry `ExtUrlQueryInfo` with `includeInRequests` containing `xlink` and the Period has `@xlink:href`, the query for
dereferencing the Period is MPD's `finalQueryString` followed by the Period's.

### I.3.5 Output (p. 306)

- `ExtUrlQueryInfo`: append as in I.2.3 step 4; percent-encode (RFC 3986) as a last step when needed.
- `ExtHttpHeaderInfo`: replace each `&` in `finalQueryString` by `", "` (comma, space) and send the result as the value of the custom header
  `MPEG-DASH-Param`.

## Fresh minimal example (ours, for tests)

```xml
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S" minBufferTime="PT2S"
     profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
  <Period>
    <AdaptationSet mimeType="video/mp4" segmentAlignment="true" startWithSAP="1">
      <EssentialProperty schemeIdUri="urn:mpeg:dash:urlparam:2014" xmlns:up="urn:mpeg:dash:schema:urlparam:2014">
        <up:UrlQueryInfo queryTemplate="auth=$query:auth$" useMPDUrlQuery="true"/>
      </EssentialProperty>
      <SegmentTemplate duration="5" startNumber="1" media="seg_$Number$.mp4" initialization="init.mp4"/>
      <Representation id="r1" codecs="avc1.4d401f" width="640" height="360" bandwidth="800000"/>
    </AdaptationSet>
  </Period>
</MPD>
```

With MPD URL `https://cdn.test/m.mpd?auth=abc&x=1` the first Segment request is `https://cdn.test/seg_1.mp4?auth=abc`.
