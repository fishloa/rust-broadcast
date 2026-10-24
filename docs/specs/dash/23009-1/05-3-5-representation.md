<a id="c5-3-5"></a>

# ISO/IEC 23009-1:2022 clause 5.3.5 - Representation (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.5, printed pp. 51-62 (PDF 59-70). Verbatim, page-verified text, Tables 9-12 and XML syntax:
`private/specs-verified/dash/23009-1/05-3-5-representation.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-5-1"></a>

## 5.3.5.1 Overview and conformance rules (p. 51-52)

- A Representation is one alternative encoding of the complete or partial set of content components of a Period. It starts at PeriodStart and runs to the end of the Period. It contains one media stream per component and consists of Segments.
- Initialization rule: each Representation either has an Initialization Segment, or every Media Segment is self-initializing (itself conforms to the `@mimeType` media type).
- **Independent Representation** (no `@dependencyId`): Initialization Segment (if any) plus all consecutive Media Segments form a conforming Segment track (4.5.4) in the `@mimeType` format.
- **Dependent Representation** (has `@dependencyId` listing the `@id`s of its complementary Representations): for dependent X and complementary Y, Subsegment m of X and Subsegment n of Y do not
  overlap when m differs from n. Initialization Segment followed by, for each Subsegment of X, the matching Subsegments of each complementary Representation in `@dependencyId` order and then X's own,
  forms a conforming Subsegment track.
  Consequences stated in notes: starting at a SAP in (Sub)Segment i needs no complementary data from before number i; when dependencies reach a shared base Representation by several paths, that base's
  (Sub)Segments are concatenated only the first time they are met following the `@dependencyId` order from X.
- **Associated Representation** (`@associationId`, optional `@associationType`): supplementary or descriptive relation; its Segments may be optional for decoding/presenting the associated targets. Association is only allowed between Representations that are not in the same Adaptation Set.
- For dynamic MPDs, means to compensate (clock) drift are recommended (Annex A.8).

<a id="c5-3-5-2"></a>

## 5.3.5.2 Representation semantics (Table 9, p. 52-55)

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@id` | M | string, no whitespace | - | Unique within a Period unless the Representation is functionally identical to another one in the Period. |
| (cont.) |||| If used in template URL construction (5.3.9.4.4) only RFC 3986 HTTP-URL characters are permitted. (Analysis: [`dash-23009-1-2022-ids.md`](dash-23009-1-2022-ids.md).) |
| `@bandwidth` | M | unsigned int (bps) | - | Peak-rate model: with a constant-rate channel of this rate, starting from any SAP signalled by `@startWithSAP` or any Segment Index, playout can be continuous if it begins after `@minBufferTime * @bandwidth` bits arrived (i.e. |
| (cont.) |||| `@minBufferTime` after the first bit). For a dependent Representation the value covers it plus all complementary Representations. See 5.3.5.4. |
| `@qualityRanking` | O | unsigned int | none | Rank within the same Adaptation Set; lower value = higher quality. |
| `@dependencyId` | O | list of ids | none (independently decodable) | `@id`s of the complementary Representations; must not appear when there are no dependencies. |
| `@associationId` | O | list of ids | none | `@id`s of associated Representations. |
| `@associationType` | O | list of 4CCs | none | One MP4-registered track-reference 4CC per `@associationId` entry; forbidden without `@associationId`; the counts must match. |
| `@mediaStreamStructureId` | O | list of strings | none (no known similarity) | Video only. |
| (cont.) |||| Streams sharing a value must have the same number of SAPs of types 1-3, identical T_SAP/T_DEC/T_EPT/T_PTF for the i-th such SAP, and splicing stream one up to I_SAU (exclusive) of the i-th SAP with stream two from that I_SAU must give a conforming stream of acceptable quality. |
| (cont.) |||| Values must differ between Adaptation Sets. |
| (cont.) |||| Multiple values per Representation express partial switching permissions (example: A "1", B "1 2", C "2" allows A-B and B-C switching but not A-C). |
| CommonAttributesElements | - | - | - | `RepresentationBaseType`, 5.3.7. |
| `BaseURL` | 0..N | - | - | 5.6. |
| `ExtendedBandwidth` | 0..N | - | - | Extended bandwidth model, 5.3.5.6. |
| `SubRepresentation` | 0..N | - | - | Embedded sub-representation, 5.3.6. |
| `SegmentBase`, `SegmentList`, `SegmentTemplate` | 0..1 each | - | - | Segment information (5.3.9); most specific level wins. |

<a id="c5-3-5-3"></a>

## 5.3.5.3 Element tree and simple types

```text
Representation : RepresentationBaseType + { @id @bandwidth @qualityRanking @dependencyId
                 @associationId @associationType @mediaStreamStructureId }
 +- BaseURL* +- ExtendedBandwidth* +- SubRepresentation* +- SegmentBase? +- SegmentList? +- SegmentTemplate?
```

`@id` is `StringNoWhitespaceType` (pattern excluding whitespace and Unicode separators), required; `@bandwidth` and `@qualityRanking` are `xs:unsignedInt`; `@dependencyId`, `@associationId`, `@mediaStreamStructureId` are whitespace-separated
string lists (`StringVectorType`); `@associationType` is a list of `FourCCType` (an unrestricted string). Ed.6 differences: [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

<a id="c5-3-5-4"></a>

## 5.3.5.4 Bandwidth and minimum buffer time (p. 55-56)

- Pair: MBT = `MPD@minBufferTime`, BW = `Representation@bandwidth`. MBT does not tell a client how much to buffer; it describes how much buffer ideal-network playback needs, i.e. the burstiness of the *encoding* (the size of the "leaky bucket" that makes BW true). It is a content property.
- Guarantee (formula form): if the Representation, starting at any Segment, is delivered at constant rate BW, every access unit with presentation time PT arrives no later than PT + MBT.
- Absent other guidance, set MBT to the maximum GOP (coded video sequence) size, often equal to the maximum Segment duration (live profile) or Subsegment duration (on-demand). It may be smaller than the maximum (sub)segment duration but should not be larger.

<a id="c5-3-5-5"></a>

## 5.3.5.5 Random access signalling (Tables 10-11, p. 56-58)

Random access = start processing at an access point at time t: initialise from the Initialization Segment (if any) and decode from the signalled Segment onwards. Optional `RandomAccess` element (child of the common group):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@interval` | M | unsigned int, `@timescale` units | - | Spacing of random access points. |
| (cont.) |||| A Segment whose MPD start time minus the `@t` of its `S` element is an integer multiple of the interval (the spec phrases it as a multiple of the product of `@timescale` and `@interval`) is a random access opportunity under strategy `@type`. |
| (cont.) |||| Pick it so times coincide with Segment starts, otherwise nothing is described. |
| `@type` | OD | `closed` / `open` / `gradual` | `closed` | Strategy; a client must ignore the whole element if the value is unknown. |
| `@minBufferTime` | O | duration | inherited from MPD | As in 5.3.5.4 but for these access points. |
| `@bandwidth` | O | unsigned int | inherited from Representation | Peak-rate model starting at any RAP listed here; dependent-Representation aggregation as in Table 9. |

Strategies (Table 11): `closed` - the Segment is a Random Access Segment and starts with SAP type 1 or 2; `open` - starts with SAP type 1, 2 or 3; `gradual` - starts with type 1, 2, 3 or 4. The SAP type is necessary but not sufficient: every Random Access Segment requirement must also hold.
Schema: `RandomAccessType` with `@interval` (unsignedInt, required), `@type` (`RandomAccessTypeType` enumeration `closed|open|gradual`, default `closed`), `@minBufferTime` (duration), `@bandwidth` (unsignedInt).

<a id="c5-3-5-6"></a>

## 5.3.5.6 Extended bandwidth signalling (Table 12, p. 58-60)

The single MBT/BW pair cannot express VBR nature or bitrate over other time windows, so each Representation may carry `ExtendedBandwidth` elements:

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `ExtendedBandwidth@vbr` | OD | boolean | `false` | True: the encoding aims at consistent quality and only drops quality when the MBT/BW limits bind; instantaneous bitrate is often below the maximum. False or absent: nothing is said. |
| `ModelPair` | 0..N | - | - | One extra (window, rate) model. |
| `ModelPair@bufferTime` | M | duration | - | Window for this pair. Usable for long-term averages: on-demand average (window = Period duration, bandwidth = Representation size) or maximum expected long-term rate at a live edge (window such as 1 h or 24 h). |
| `ModelPair@bandwidth` | M | unsigned int (bps) | - | Peak-rate model using `@bufferTime` instead of MBT, valid starting at any signalled SAP of type 1 or 2 in this element; dependent-Representation aggregation as above. |

Schema: `ExtendedBandwidthType` = `ModelPair*` + any-other + `@vbr` (boolean, default false); `ModelPairType` has required `@bufferTime` (duration) and `@bandwidth` (unsignedInt). These are unchanged in Ed.6 ([`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md)).

## Transcription notes (spec quirks)

- Table 10 `@minBufferTime` refers to "subclauses 0 and 5.3.5.4": the first is a broken cross-reference field (p. 57).
- 5.3.5.6 has the typo "each Represention have assigned" (p. 58); the Table 12 `@bufferTime` note writes "@buffer" where `@bufferTime` is meant (p. 59).
- Table 9 `@bandwidth` mentions `@startWithSAP`, which is defined in the common-attribute table (5.3.7, Table 14), not here (p. 53).
- The sub-clause numbering in the Table 9 `ExtendedBandwidth`/`SubRepresentation` rows both point to "5.3.5.6"; Sub-Representation is actually clause 5.3.6 (p. 54-55).
