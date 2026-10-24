---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex C (normative) - MIME type registration for MPD and other resources, C.1 to C.5"
pdf_pages: "243-249"
printed_pages: "235-241"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-c-mime.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex C - MIME type registrations (implementer notes)

## C.1-C.2 `application/dash+xml` (pp. 235-236)

| Registration field | Value |
|---|---|
| Type / subtype | `application` / `dash+xml` |
| Required parameters | none |
| Optional parameters | `profiles` (C.3) |
| Encoding | UTF-8 |
| File extension | `mpd` |
| Intended usage | common |
| Change controller | ISO/IEC JTC1/SC29 (MPEG) |
| Published specification | ISO/IEC 23009-1 |

Security points a client should honour: the MPD is XML, so malformed input is a risk; an MPD can steer a client to fetch arbitrary other
resources (denial-of-service amplification when widely distributed); the format has no active content, but referenced resources may, and
are identified by their own types; in an insecure environment MPD parts may be protected with W3C XML Encryption and authenticated with W3C
XML Signature.

## C.3 `profiles` parameter (pp. 236-237)

- Optional. Lists the profile(s) the file claims. The value must follow the `pro-simple` or `pro-fancy` productions of RFC 6381:2011 section 4.5.
- The listed identifiers should be identical to those in `MPD@profiles` (see [08-profiles.md](08-profiles.md) for the identifier table).
- Our own example of the shape: `application/dash+xml;profiles="urn:mpeg:dash:profile:isoff-live:2011"`.

## C.4 MPD anchors (URI fragments) (pp. 237-239)

An MPD URL may carry a fragment selecting where and what to start. The fragment is an ampersand-separated list of `key=value` pairs.
Reserved characters in values are percent-coded (RFC 5986, as cited by the standard). The annex names one temporal parameter (`t`, the text
calls it "position") and two context parameters ("state" and "selection") but Table C.1 defines the four keys below.

| Key | Value type | Meaning (Table C.1, p. 238) |
|---|---|---|
| `t` (printed `T` in the table) | W3C Media Fragments "basic" time or time range; optionally a prefixed comma-separated number pair | An integer time is seconds from the start of the Period chosen by `period` (default Period: the first). Default when absent: `t=0`. With the `posix:` prefix it is an absolute UTC time range in seconds since 1970-01-01T00:00:00Z (fractions to milliseconds allowed); only valid when `MPD@availabilityStartTime` exists; must not be combined with `period` (an anchor holds at most one of `period` and `t`). Special value `now` = latest available Segment (live edge). |
| `period` | string | A `Period@id`; default is the Period with the earliest `PeriodStart` |
| `track` | string | one `AdaptationSet@id` (repeatable, see examples) |
| `group` | string | one `AdaptationSet@group` |

Addressability depends on authors setting `Period@id`, `AdaptationSet@id` and `@group` explicitly.

Example anchors (descriptions are ours; the strings are spec examples that are plain facts of syntax):

| Anchor | Effect |
|---|---|
| `my.mpd#t=42&period=Period1` | 42 s into the Period whose id is `Period1` |
| `my.mpd#t=42&track=en51&track=vid` | 42 s from presentation start, only sets with ids `en51` and `vid` |
| `my.mpd#t=60,180` | time range 60 s to 180 s |
| `my.mpd#t=posix:1421870645` | live start at that absolute UTC time |
| `my.mpd#t=posix:1421870645&track=en51&track=vid` | same, restricted to two sets |
| `my.mpd#t=posix:1421870645,1421883873` | live range between two absolute times |
| `my.mpd#t=posix:now,1421883873` | from the live edge to an absolute end |
| `my.mpd#t=posix:0,1421883873` | from the earliest available Segment to an absolute end |

### C.4.4 Resolving a `posix` range (p. 239)

Let `N` = UTC at request time, `E` = earliest available Segment time, `F` = latest available Segment time, `S` = anchor start, `T` = anchor end.

- If `T` is missing or greater than `F`, use `T = F`.
- If `S` is missing, use `S = E`. If `S` is `now`, use `S = N`.

## C.5 `application/dash-patch+xml` (pp. 239-241)

| Registration field | Value |
|---|---|
| Type / subtype | `application` / `dash-patch+xml` |
| Required / optional parameters | none / none |
| Encoding | 8 bit, always UTF-8 |
| File extension | `mpp` |
| Intended usage | common |
| Change controller | ISO/IEC JTC1/SC29 (MPEG) |
| Published specification | ISO/IEC 23009-1 (patch format introduced in 5.15) |

Security notes mirror C.2 and add: patches are built on the RFC 5261 patch framework and inherit its security considerations; a patch can
introduce foreign-namespace content into the MPD, which raises its own concerns.

Other resource types: the annex registers only these two media types. Clients decide `.mpd`/`.mpp` handling by these types (patch format:
5.15, outside this PR).

## Transcription notes

- Table C.1 was rebuilt from the render in the private copy (see VERIFICATION-C1.md).
