---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.8.1-5.8.4 - Descriptors: generic DescriptorType and the specific descriptor elements (own-words implementer notes)
clauses: 5.8.1, 5.8.2, 5.8.3, 5.8.4.1-5.8.4.12
printed_pages: 114-122
pdf_pages: 122-130
verbatim_source: private/specs-verified/dash/23009-1/05-8.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 5.8 Descriptors (generic and specific elements)

## Generic descriptor (5.8.1-5.8.3, Table 32)

Every descriptor element has the same shape and the meaning of the element is entirely defined by the scheme it names.

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| @schemeIdUri | M | xs:anyURI | - | identifies the scheme; URN or URL; a URL should embed a month-year (mmyyyy) at which the URL's owner authorised the assignment |
| @value | O | xs:string | - | value in the scheme's own value space (simple enumerations as text; structured data goes into elements of another namespace) |
| @id | O | xs:string | - | descriptors with equal `@id` are synonymous: processing one of them suffices |
| any element from another namespace | 0..N | - | - | extension content |

Rules:

- A DASH application that uses a descriptor must first define a scheme URI and then the value space for that scheme; the MPD itself says nothing on how to use them (the clause defines some schemes in 5.8.5, see [`05-8-5-schemes.md`](05-8-5-schemes.md)).
- **Equivalence of two descriptors**: same element name, equivalent `@schemeIdUri` and equivalent `@value`. URNs compare lexically (RFC 8141 clause 3); URLs compare character-for-character (RFC 3986 section 6.2.1); a missing `@value` means the schemes alone decide; extension-namespace content is ignored; `@id` may be ignored.

```text
any descriptor element
  attributes: schemeIdUri, value, id; foreign-namespace children allowed
```

## Specific descriptor elements (5.8.4)

All elements below are `DescriptorType` (ContentProtection extends it). An unrecognised scheme/value is handled per element as shown.

| Element | Purpose | Client rule on unrecognised scheme/value |
|---|---|---|
| ContentProtection | encryption / DRM information | see below |
| Role | role of a media component (one AS/component may carry several roles, even in one scheme); simple scheme in 5.8.5.5, multi-view scheme in 5.8.5.6 | - |
| Accessibility | accessibility purpose (analogous to Role); no scheme defined, the simple role scheme can give minimal information | - |
| Rating | audience suitability; no scheme defined; if an audience is restricted, content with ratings is not presented unless at least one rating scheme is recognised and says it is appropriate | - |
| Viewpoint | viewpoint annotation; Adaptation Sets with non-equivalent values hold different media content components, equivalent values are meant to be presented together (applies equally to unrecognised schemes); not limited to video; no scheme defined | - |
| FramePacking | frame-packing arrangement; several allowed, each must give enough information to select/reject; schemes of ISO/IEC 23091-3 (`VideoFramePackingType`) can be carried; this document's own schemes (5.8.5.3) are kept for compatibility | if scheme/value of all FramePacking elements is unrecognised the client is expected to ignore the described Representations (may reject the Adaptation Set) |
| AudioChannelConfiguration | audio channel layout; several allowed (e.g. MPEG Surround stereo + multichannel); ISO/IEC 23001-8 `ChannelConfiguration` can be carried; the 5.8.5.4 scheme is kept for compatibility | ignore the descriptor |
| EssentialProperty | information the client must understand to use the parent element | ignore the parent element |
| SupplementalProperty | optional optimisation information | ignore the descriptor |
| AssetIdentifier (Period level) | two Periods with equivalent asset identifiers belong to the same asset | can still be used to compare identifiers; processing the scheme is not essential |
| UTCTiming | wall-clock sources for the client; see 5.8.5.7; `@schemeIdUri` must be one of the URNs of Table 35 | clients may use any method; order = preference |
| OutputProtection | output-protection requirements; see below | - |

### EssentialProperty rules

- The author states that successful processing is essential, unless another EssentialProperty with the same `@id` is processed instead.
- Elements sharing an `@id`: processing one is sufficient; at least one element of each distinct `@id` should be processed. Several distinct ids may coexist.
- On MPD level, an `@id` group means at least one of those must be processable to access the content; if none can be, the client is expected to terminate the presentation.
- Removing an EssentialProperty must not make an MPD non-conforming to its schema and profile.

### UTCTiming (5.8.4.11)

Provides optional wall-clock time sources; if several are given their order is their preference (first = highest) but a client may pick any, accepting reduced accuracy.

### ContentProtection (5.8.4.1, Table 33)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| @schemeIdUri | M | xs:anyURI | - | content-protection / encryption scheme |
| @value | O | xs:string | - | scheme-specific info (DRM version, encryption mode, ...) |
| @robustness | OD | StringNoWhitespaceType | lowest level of the scheme | minimum robustness needed; levels, values and order are scheme specific (typically hardware vs software); a client may ignore it but then only learns suitability after the licence response; schemes defined with @robustness must list values low-to-high |
| @refId | O | xs:ID | - | identifier making this element a "source"; unique in the MPD; not allowed together with `@ref` |
| @ref | O | xs:IDREF | - | makes this a "referencing" element that inherits from the source with matching `@refId`; not allowed together with `@refId` |
| inherited from DescriptorType | - | - | - | `@id`, extension content |

Rules:

- No ContentProtection element in scope = the content is not encrypted.
- ContentProtection is an extension point: DRM-specific schemes (UUID-style URNs) add their own information; schemes of 5.8.5.2 (`mp4protection`, `CA_descriptor`) only name the encryption mode, so DRM-specific elements should be provided alongside.
- @robustness is meaningful only for schemes outside 5.8.5.2 (it is not defined for those two).
- Referencing (5.8.4.1.3): exists to avoid repeating large payloads (e.g. a `pssh`). If any element has `@ref`, exactly one element with the matching `@refId` must exist. The referencing element inherits all attributes and elements of the source, and anything it carries itself stays; the source must be in a Period not later than the Period holding the referencing element. Informative warnings: a client that cannot dereference may reject the Representations or ask the DRM server; a source may sit in a Period that has already left the time-shift buffer while still listed in MPD updates.
- Defining a ContentProtection scheme (5.8.4.1.6): required = unique `@schemeIdUri`, and the list of permitted `@value` values with meanings (none needed if the URI alone is enough). Recommended = how `@robustness` is used with ordered values, example value discovery through common APIs (W3C EME, Android), how players use the values in existing workflows, other important information. This document defines no scheme in that sense.

```text
ContentProtection = descriptor element plus attributes: robustness, refId, ref
```

### OutputProtection (5.8.4.12)

- Purpose: signal that the rendered output of the decoder must be protected on its way to the display (e.g. HDCP on HDMI) so a client can pick/prefetch Representations without waiting for the licence; the DRM licence response stays authoritative. If the element is absent the client should make no assumption.
- Defining a scheme: required = unique `@schemeIdUri` and the permitted `@value` list **in non-decreasing order** with interpretations; recommended = example value discovery and example rejection of unplayable Representations. Scheme defined here: HDCP (5.8.5.14).

## Quirks

- 5.8.4.1.1 cites "Table 30" for the ContentProtection semantics; the table is Table 33.
- `@refID` (5.8.4.1.3 text) vs `@refId` (table/syntax); `@schemeIdURI` spelled with capital URI in 5.8.4.1.1 and 5.8.4.11.
- 5.8.4.1.2 note: different DRMs and different codecs can have different robustness levels on one device.

Verbatim text: see `private/specs-verified/dash/23009-1/05-8.md` sections 5.8.1-5.8.4 (printed pp. 114-122).
