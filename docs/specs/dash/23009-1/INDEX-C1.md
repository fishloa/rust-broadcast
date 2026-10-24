# ISO/IEC 23009-1:2022 (DASH, 5th edition) - clause 8 and annexes, implementer notes (branch C1)

Original-wording implementer notes for **clause 8 (Profiles)** and **Annexes A, B, C, E, G, I, K** of ISO/IEC 23009-1:2022(E)
(`private/specs/iso_iec_23009-1_dash_2022_ed5.pdf`, 330 pages; printed page = PDF page - 8), plus honest status entries for Annexes D, F, H, J.

Policy for this tree: the public files here are **our own explanatory notes** (identifiers, numbers, URNs and formulas are facts and are
given exactly; prose is paraphrased; no spec XML syntax blocks or spec example documents are copied). The **verbatim** text is the output of the
`pdf2md` tool over the owner's licensed PDF and lives only in the private submodule under `private/specs-verified/dash/23009-1/`
(branch `dash-specs-C1` of the private repo, not pushed, submodule pointer not bumped in this PR). How each page range was verified and
every defect found: [`VERIFICATION-C1.md`](VERIFICATION-C1.md).

This file is `INDEX-C1.md` (not `INDEX.md`) so an integrator can merge it with the sibling branches' indexes without conflict.

## Clause map

| Clause | Public notes (original wording) | Private verbatim file (`private/specs-verified/dash/23009-1/`) | Printed pp. | PDF pp. |
|---|---|---|---|---|
| 8 Profiles (8.1 to 8.12.7) | [`08-profiles.md`](08-profiles.md) | `08-profiles.md` | 178-209 | 186-217 |
| Annex A Example client behaviour (A.1 to A.13) | [`annex-a-client-behaviour.md`](annex-a-client-behaviour.md) | `annex-a-client-behaviour.md` | 210-233 | 218-241 |
| Annex B MPD schema | [`annex-b-schema.md`](annex-b-schema.md) | `annex-b-schema.md` | 234 | 242 |
| Annex C MIME registrations (C.1 to C.5) | [`annex-c-mime.md`](annex-c-mime.md) | `annex-c-mime.md` | 235-241 | 243-249 |
| Annex D DASH Metrics | status below | `annex-d-metrics.md` | 242-250 | 250-258 |
| Annex E Byte range via plain GET | [`annex-e-byte-range.md`](annex-e-byte-range.md) | `annex-e-byte-range.md` | 251-253 | 259-261 |
| Annex F Extending DASH to other delivery formats | status below | `annex-f-extending.md` | 254-255 | 262-263 |
| Annex G MPD examples and usage (G.1 to G.22) | [`annex-g-examples.md`](annex-g-examples.md) | `annex-g-examples.md` | 256-283 | 264-291 |
| Annex H Spatial Relationship Description | status below (value syntax summarised) | `annex-h-srd.md` | 284-292 | 292-300 |
| Annex I Flexible insertion of URL parameters | [`annex-i-urlparam.md`](annex-i-urlparam.md) | `annex-i-urlparam.md` | 293-306 | 301-314 |
| Annex J Open GOP resolution change | status below | `annex-j-open-gop.md` | 307 | 315 |
| Annex K DASH Service Description | [`annex-k-service-description.md`](annex-k-service-description.md) | `annex-k-service-description.md` | 308-318 | 316-326 |

### Sibling coverage {#sibling-coverage}

Clauses 3, 4 and 5.1 to 5.3.9 are in PR #1516 (branch `docs/dashspecA`: `03-terms.md` through `05-3-9-7-failover-content.md`, `INDEX.md`,
`VERIFICATION.md`, `ED6-SCHEMA-DIFF.md`). Those files are not in this branch; links to them in these notes resolve once the branches are
merged. Not transcribed by anyone yet (outside this task): clauses 5.3.10 onward, 5.4 to 5.16 (apart from what Annex notes quote), 6
(Segment formats) and 7 (MPD and Segment constraints for ISO BMFF / MPEG-2 TS). Clause 8 and Annex A rely on those clauses.

### Fresh test inputs

Nine new example MPDs (F1 to F9) and small illustrations are in [`annex-g-examples.md`](annex-g-examples.md), [`annex-i-urlparam.md`](annex-i-urlparam.md)
and [`annex-k-service-description.md`](annex-k-service-description.md). All MPD examples were checked against the Ed.6 `DASH-MPD.xsd` with
`xmllint --schema` (the patch document F8 could only be checked for well-formedness because `DASH-MPD-PATCH.xsd` includes a network schema).
The spec's own examples are available line-structured in `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/example_*.mpd`.

## Status entries for annexes not given notes

### Annex D (normative) - DASH Metrics, printed pp. 242-250

What it is: a client QoE metric model. Reference model with three observation points (network/client, client/application, application/output),
and the semantics of the metrics a service can ask a client to collect: TCP connection list, HTTP request/response transaction list (with
trace of throughput samples), Representation switch events, buffer level, play list (playback events and trick modes), device information.
Reporting transport and the MPD `Metrics`/`Reporting` elements belong to clause 5.9 (not covered here).
Why not given notes now: not needed to *play* a stream. **Honest relevance:** a full client that supports MPD-requested metrics reporting
(`Metrics` elements) must implement the D.4 keys; a pure playback client may ignore all of it. The private verbatim file exists for a later pass.

### Annex F (informative) - Guidelines for extending DASH with other delivery formats, printed pp. 254-255

What it is: authoring guidance for adding a new container to DASH. A delivery format should let any portion be decoded using only a constant
amount more data than that portion (a header plus independently playable blocks); an index (such as `sidx`/`ssix`) located without
downloading the whole Segment is desirable; a format specification should define the MIME type of a Representation as concatenated
Segments, describe self-initializing or Initialization+Media Segment formats, optionally Index Segments, bitstream switching Segments, SAP
interpretation and the container-specific meaning of `@bitstreamSwitching`, `@segmentAlignment`, `@subsegmentAlignment`. When MPD attributes
are repeated in the media, a mismatch should not occur and the media value takes precedence for decoding.
Why not given a separate file: informative and about authoring new formats; not needed by a playback client for ISO BMFF / MPEG-2 TS. The one
client-relevant rule (media values win over MPD values for decoding) is stated above.

### Annex H (normative) - Spatial Relationship Description, printed pp. 284-292

What it is: descriptors that describe how an Adaptation Set or Sub-Representation (a "Spatial Object") sits in a larger source frame
(tiles, regions of interest). **Relevant to tiled/panoramic clients; ignorable by plain clients** because SRD is carried in
`SupplementalProperty`/`EssentialProperty`, and authors must keep at least one Representation playable without any `EssentialProperty` SRD.
Value syntax (own summary; identifiers and parameter names are facts):

| Scheme | URN | `@value` (comma separated) |
|---|---|---|
| static | `urn:mpeg:dash:srd:2014` | `source_id, object_x, object_y, object_width, object_height [, total_width, total_height [, spatial_set_id]]`, all non-negative decimal integers; `total_width` and `total_height` come together; at least one descriptor per `source_id` per Period must give them; each object must fit in the reference space (`object_x + object_width <= total_width`, same vertically); `spatial_set_id` requires the totals |
| dynamic | `urn:mpeg:dash:srd:dynamic:2016` | `source_id, coordinate_id [, spatial_set_id]` where `coordinate_id` is the `@id` of a Representation carrying a `2dcc` timed-metadata track (ISO/IEC 23001-10) whose `@associationId` points at the moving object's Representation and `@associationType` is `cdsc` |

Mapping of `2dcc` sample fields: `top_left_x`->`object_x`, `top_left_y`->`object_y`, `width`->`object_width`, `height`->`object_height`,
`reference_width`->`total_width`, `reference_height`->`total_height`. Comparison across descriptors with the same `source_id` is done after dividing by the
totals. A descriptor with no `@value` carries no spatial information and can be ignored. Worked examples (zoomed video, tiled video, tiled panorama with a
moving region of interest) are in H.3 of the private verbatim file; full notes were not written in this task (gap).

### Annex J (informative) - Open GOP resolution change, printed p. 307

What it is: a `SupplementalProperty` scheme `urn:mpeg:dash:resolutionSwitching:2016` (Adaptation Set or Representation level only) that tells
a client which Representations allow seamless resolution switching at the start of any Segment beginning with a SAP of type 1 to 3.
`@value` is a white-space separated pair: `switchableTo` (M, comma-separated list of `@id` of the target Representations, whose switch points
are open-GOP type-3 SAPs) and `switchingMimeType` (M, comma-separated MIME types of the stream formed by the current Representation's
Initialization Segment plus consecutive Segments followed by Segments from the target). The descriptor must not be present unless every access
unit of Segment N with presentation time inside the interval from the earliest presentation time to the first decodable time depends only on
Segments N or N-1. A switching client may need to decode an additional stream during Segment N-1 (the one named by `switchingMimeType`).
Relevance: only for a client that implements open-GOP seamless resolution switching; safely ignorable otherwise since it is a
`SupplementalProperty`. Notes were short enough to be given here in full; no separate file.
