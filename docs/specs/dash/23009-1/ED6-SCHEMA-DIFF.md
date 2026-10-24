# XML schema fragments: Ed.5 (2022 print) vs Ed.6 (2026 schema files)

Not Ed.6 semantics: **the Ed.6 prose is paywalled and was not read**; this note compares only the *schema syntax blocks transcribed
from the Ed.5 PDF* (the `xml` fences of the clause files) with `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/DASH-MPD.xsd`
(the electronic insert published with Ed.6). Nothing in the Ed.6 file is adopted in the clause files; where Ed.5 and Ed.6 differ, Ed.5 is
what the transcription says. Remember that Ed.5 Annex B (the normative schema of Ed.5) was not part of this transcription range; the
fragments below are the clause-5 snippets, which the spec says are subordinate to Annex B.

Method: both sides parsed with lxml; for each of the 39 `complexType`/`simpleType` declarations transcribed from Ed.5, the child
elements, attributes (name, type, use, default, fixed), `any`/`anyAttribute` and restriction facets were compared with the Ed.6
declaration of the same name. 29 types are structurally identical; 10 differ as listed. No Ed.5 type is missing from Ed.6.

| Type (Ed.5 file) | Ed.5 (transcribed) | Ed.6 file |
|---|---|---|
| `MPDtype` ([05-3-1](05-3-1-hierarchy.md)) | `Location` is `xs:anyURI`; no `ContentSteering`, no `RequestParam` | `Location` is `LocationType`; adds `ContentSteering` (`ContentSteeringType`, 0..1) and `RequestParam` (`up:ExtendedUrlInfoType`, 0..unbounded) |
| `PresentationType` ([05-3-1](05-3-1-hierarchy.md)) | enumeration `static`, `dynamic` | adds enumeration value `list` |
| `ListOfProfilesType` ([05-3-1](05-3-1-hierarchy.md)) | pattern built from entities `(&URN;\|&URL;)(&comma_sep;(&URN;\|&URL;))*` | the same pattern with the entities expanded inline (the expansion was not compared character by character; it may be the same language) |
| `PeriodType` ([05-3-2](05-3-2-period.md)) | no `ImportedMPD`, no `RequestParam`, no `@minBufferTime` | adds `ImportedMPD` (`ImportedMpdType`, 0..1), `RequestParam` (`up:ExtendedUrlInfoType`, 0..unbounded), attribute `minBufferTime` (`xs:duration`) |
| `AdaptationSetType` ([05-3-3](05-3-3-adaptation-set.md)) | no `RequestParam` | adds `RequestParam` (`up:ExtendedUrlInfoType`, 0..unbounded) |
| `RepresentationType` ([05-3-5](05-3-5-representation.md)) | no `RequestParam` | adds `RequestParam` (`up:ExtendedUrlInfoType`, 0..unbounded) |
| `RepresentationBaseType` ([05-3-7](05-3-7-common.md)) | no `SegmentSequenceProperties` | adds `SegmentSequenceProperties` (`SegmentSequencePropertiesType`, 0..unbounded) |
| `CodecsType` ([05-3-7](05-3-7-common.md)) | two patterns built from entities (`&charset;&squote;&language;&squote;&id_list;`, `&id_simple;(,&id_simple;)*`) | two patterns with the entities expanded inline (not compared character by character) |
| `MultipleSegmentBaseType` ([05-3-9](05-3-9-segment-information.md)) | `@startNumber`, `@endNumber` are `xs:unsignedInt` | both are `xs:unsignedLong`; adds `@endSubNumber` (`xs:unsignedLong`), `@k` (`xs:unsignedLong`, default 1), `@tolerance` (`xs:float`, default 50.0) |
| `SegmentTimelineType` ([05-3-9-6](05-3-9-6-segment-timeline.md)) | `S` 0..unbounded; `S@d` required; `S@k` is `xs:unsignedLong` (default 1) | adds `Pattern` (`PatternType`, 0..unbounded); `S` is 1..unbounded; `S@d` is optional; `S@k` is `xs:unsignedInt` (default 1); adds `S@p` (`xs:unsignedInt`), `S@pE` (`xs:unsignedInt`, default 0), `S@ssp` (`xs:unsignedInt`) |

The `up:` prefix is the URL-parameter schema namespace of the Ed.6 insert (`DASH-MPD-UP.xsd`), which is not part of the Ed.5 clause-5 snippets.

Identical in both editions (as transcribed): `RFC6838ContentTypeType`, `SAPType`, `RatioType`, `FrameRateType`, `SwitchingType`,
`SwitchingTypeType`, `RandomAccessType`, `RandomAccessTypeType`, `ExtendedBandwidthType`, `ModelPairType`, `ContentComponentType`, `TagType`,
`StringNoWhitespaceType`, `StringVectorType`, `ListOf4CCType`, `FourCCType`, `UIntVWithIDType`, `UIntVectorType`, `SubRepresentationType`,
`SubsetType`, `SegmentBaseType`, `URLType`, `SingleRFC7233RangeType`, `SegmentListType`, `SegmentURLType`, `SegmentTemplateType`,
`FailoverContentType`, `AudioSamplingRateType`, `VideoScanType`. The identical set is a structural finding (names, types, use, default,
fixed, facets), not a statement about the Ed.6 prose.
