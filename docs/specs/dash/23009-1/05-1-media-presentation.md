<a id="c5"></a>

# ISO/IEC 23009-1:2022 clauses 5.1-5.2 - Media Presentation and the MPD (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), clauses 5.1 and 5.2, printed pp. 22-26 (PDF 30-34). Verbatim, page-verified text:
`private/specs-verified/dash/23009-1/05-1-media-presentation.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-1"></a>

## 5.1 General (p. 22)

A Media Presentation is the data a client can reach to give a streaming service; it is described by an MPD plus any later MPD updates. Where the rest of clause 5 lives:
5.2 the MPD, 5.3 the data model, 5.4 MPD update, 5.5 assembling a fragmented MPD, 5.6 URL formats and request construction, 5.7 program information, 5.8 descriptors,
5.9 DASH metrics, 5.10 events and event streams, 5.11 MPD chaining, 5.12 producer reference time, 5.13 leap seconds, 5.14 content popularity rates.
(The paragraph itself points the data model at "5.2.3.5"; see the notes at the end.)

<a id="c5-2"></a>

## 5.2 Media Presentation Description

<a id="c5-2-1"></a>

### 5.2.1 General conformance rules for an MPD (p. 22-23)

Normative behaviour, restated:

- An MPD is a single XML document with exactly one root `MPD` element, valid against the Annex B schema. The MPD contains what a client needs to build Segment URLs and offer the service;
  it does not drive playback itself (that is the media engine's job on the Representation data).
- The DASH namespace is reserved to ISO/IEC. Authoring constraint with two halves: (a) strip every DASH-namespace element/attribute that is not in the Annex B schema and the result must
  still be valid and conforming; (b) strip every element/attribute from other namespaces and the result must also still be valid and conforming. Consequence for a client: removing unknown
  DASH-namespace and foreign-namespace content yields a usable, conforming MPD, so an older client can safely ignore newer additions.
- A profile (clause 8) can add authoring rules and may allow ignoring some elements; this never relaxes the general conformance rules above.
- MIME type: Annex C. Encoding: UTF-8 (RFC 3629) for the MPD and all extension-namespace data; binary data must be Base64 (RFC 4648) inside a properly namespaced UTF-8 element.
- Delivery is out of scope; over HTTP the document may be transfer-encoded (for example gzip, RFC 1952). Encryption (W3C XML Encryption) and signing (W3C XML Signature) of the MPD are possible
  but not normative here.
- Annex G has examples; the URL syntax and request-building process is 5.6.

<a id="c5-2-2"></a>

### 5.2.2 Schema (p. 23)

- The normative schema is Annex B. If it disagrees with the clause-5 syntax snippets or with prose, Annex B wins.
- ISO publishes the schema and the Annex G examples as downloadable files at the standards.iso.org "ed-5" location (`https://standards.iso.org/iso-iec/23009/-1/ed-5/en`).
- Schema root facts: target and default namespace `urn:mpeg:dash:schema:mpd:2011`; `elementFormDefault="qualified"`, `attributeFormDefault="unqualified"`; imports the W3C XLink schema; global
  element `MPD` of type `MPDtype`.

<a id="c5-2-3"></a>

### 5.2.3 Additions by edition

<a id="c5-2-3-1"></a>

#### 5.2.3.1 Purpose (p. 24)

The additions are tracked so one MPD can serve clients implementing different editions, by applying the 5.2.1 stripping rule.

<a id="c5-2-3-2"></a>

#### 5.2.3.2 Added in the 2nd edition (2014), relative to the 2012 edition with Cor 1 (p. 24)

`MPD@publishTime`; `MPD.EssentialProperty`; `MPD.SupplementalProperty`; `Period.AssetIdentifier`; `Period.EventStream`; `Period.SupplementalProperty`;
`RepresentationBase.InbandEventStream`; `SegmentBase@availabilityTimeOffset`; `SegmentBase@availabilityTimeComplete`; `BaseURL@availabilityTimeOffset`;
`BaseURL@availabilityTimeComplete`; `Subset@id`; `SegmentTimeline.S@n`.

<a id="c5-2-3-3"></a>

#### 5.2.3.3 Added in the 3rd edition (2019), relative to 2014 (p. 24-25)

`MPD.UTCTiming`; `Period.GroupLabel`; `Period.Preselection`; `Period.EmptyAdaptationSet`; `RepresentationBase.Switching`; `RepresentationBase.RandomAccess`;
`RepresentationBase.GroupLabel`; `RepresentationBase.Label`; `RepresentationBase@selectionPriority`; `Representation@tag`; `Representation@associationId`;
`Representation@associationType`; `SegmentBase@presentationDuration`; `SegmentBase@timeShiftBufferDepth`; `SegmentTimeline.S@n`; `SegmentTimeline.S@k`.
(`SegmentTimeline.S@n` appears in both this list and the 2014 list as printed.)

<a id="c5-2-3-4"></a>

#### 5.2.3.4 Added in the 4th edition, relative to 2019 (p. 25-26)

`MPD.ServiceDescription`; `MPD.InitializationSet`; `MPD.InitializationGroup`; `MPD.InitializationPresentation`; `MPD.LeapSecondInformation`; `Period.ServiceDescription`;
`EventStream@presentationTimeOffset`; `Event@contentEncoding`; `AdaptationSet@initializationSetRef`; `RepresentationBase.ProducerReferenceTime`;
`RepresentationBase.ContentPopularityRate`; `Preselection@order`; `SegmentBase.FailoverContent`; `SegmentBase@eptDelta`; `MultiSegmentBase@endNumber`; `BaseURL@timeShiftBufferDepth`.

<a id="c5-2-3-5"></a>

#### 5.2.3.5 Added in the 5th edition (2021 revision), relative to 2020 (p. 26)

`MPD.ContentProtection`; `MPD.PatchLocation`; `Period.ContentProtection`; `AdaptationSet@initializationPrincipal`; `RepresentationBase.Resync`; `RepresentationBase@containerProfiles`;
`Representation.ExtendedBandwidth`; `ContentProtection@ref`; `ContentProtection@refId`; `ContentProtection@robustness`; `RepresentationBase.OutputProtection`.

## Transcription notes (spec quirks)

- 5.1 (p. 22) says the data model is defined in "5.2.3.5"; that subclause is the list of fifth-edition additions. The data model is clause 5.3.
- The heading of 5.2.3.4 carries no year ("ISO/IEC 23009-1") in print (p. 25).
- `SegmentTimeline.S@n` is listed as added both in the 2nd edition (5.2.3.2) and the 3rd (5.2.3.3).
