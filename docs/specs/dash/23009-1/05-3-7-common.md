<a id="c5-3-7"></a>

# ISO/IEC 23009-1:2022 clause 5.3.7 - common attributes and elements (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), 5.3.7, printed pp. 63-70 (PDF 71-78). Verbatim, page-verified text, Table 14 and XML syntax:
`private/specs-verified/dash/23009-1/05-3-7-common.md`. See [`INDEX.md`](INDEX.md).

<a id="c5-3-7-1"></a>

## 5.3.7.1 Overview and the inheritance rule (p. 63)

`AdaptationSet`, `Representation` and `SubRepresentation` all derive from one base (`RepresentationBaseType`) and may carry the attributes and elements of Table 14. Use-column rule: an attribute marked M must be *available for each Representation*, meaning it is
on the Representation itself or, failing that, on the containing AdaptationSet; an O attribute may be absent at both levels. (5.3.3.1 adds that each common attribute may appear at one level only, never both.)

<a id="c5-3-7-2"></a>

## 5.3.7.2 Semantics (Table 14, p. 63-68)

Attributes:

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| `@profiles` | O | profile list | inherited from next higher level | Profiles the Representation(s) conform to; must be a subset of the values at every higher level (Representation, AdaptationSet, MPD). Same syntax as `MPD@profiles` (5.3.1.2). |
| `@width`, `@height` | O | unsigned int | unknown | Horizontal and vertical visual presentation size of video, on a grid determined by `@sar`; if `@sar` is absent they are specified as if it were "1:1". |
| (cont.) |||| The size is the sample count after cropping by encoded cropping parameters, overscan signalling or pan/scan parameters (a NOTE in the table). Unknown if absent at every level. |
| (cont.) |||| (Analysis: [`dash-23009-1-2022-aspect.md`](dash-23009-1-2022-aspect.md).) |
| `@sar` | O | ratio `x:y` | unknown | Sample aspect ratio of the video: first number = horizontal size of an encoded sample, second = vertical size, in common arbitrary units (example "10:11"). Unknown if absent at every level. |
| `@frameRate` | O | `F` or `F/D` | unknown | Output frame rate (half the field rate for interlaced); if variable, the average over the entire Representation. Value is F/D per second, D defaulting to 1. |
| `@audioSamplingRate` | O | one or two unsigned ints | unknown | Samples per second; a single value, or a whitespace-separated minimum and maximum pair. |
| `@mimeType` | M | string | - | MIME type of the concatenation of the Initialization Segment (if any) and all consecutive Media Segments of the Representation. |
| `@segmentProfiles` | O | 4CC list (RFC 6381 4.5 pro-simple/pro-fancy, no quotes) | derivable from `@profiles` | Segment profiles essential to process the Representation; meaning depends on `@mimeType`; ids are the Segment brands of clause 6; 4CCs needing XML escaping follow XML 1.0 (2008) 2.4. |
| `@codecs` | O | RFC 6381 simp-list or fancy-list, no quotes | - | Codecs present, including profile and level where applicable. Mandatory in practice for segment formats of this document ("shall be present"); identifiers use the RFC 6381 namespace mapping for the media format. |
| `@containerProfiles` | O | 4CC list | - | Container profiles essential to process the Representation; same syntax and escaping rules as `@segmentProfiles`. |
| `@maximumSAPPeriod` | O | double (seconds) | unknown | Largest interval between the T_SAP of any two successive SAPs of types 1-3 of one stream in the associated Representations. |
| `@startWithSAP` | O | SAP type 0..6 | unknown | When above 0, every Media Segment of every stream starts with a SAP of type at most this value. ("Starts with" is as for `@subsegmentStartsWithSAP`.) |
| `@maxPlayoutRate` | O | double | `1` | Highest playout multiple supported with the same decoder profile/level as normal playout. |
| `@codingDependency` | O | boolean | not stated | True: at least one access unit depends on others for decoding. False: none does (for video, all pictures intra coded). Unspecified: may or may not. |
| `@scanType` | O | `progressive` / `interlaced` / `unknown` | `progressive` (if unspecified at every level) | Scan type of the source material. |
| `@selectionPriority` | OD | unsigned int | `1` | Selection priority; with no other information higher numbers are preferred. |
| `@tag` | O | string | none | Tag usable for selection toward the decoder; mainly for Preselections and Adaptation Sets, other uses not precluded. |

Child elements (occurrence in brackets):

| Element | Occurs | Gloss and reference |
|---|---|---|
| `FramePacking` | 0..N | Frame-packing arrangement of video; if absent, no frame packing is used (5.8.4.6). |
| `AudioChannelConfiguration` | 0..N | Audio channel configuration (5.8.4.7). |
| `ContentProtection` | 0..N | Protection schemes (5.8.4.1). |
| `OutputProtection` | 0..1 | Output protection required to present the Representations (5.8.4.12). |
| `EssentialProperty` / `SupplementalProperty` | 0..N each | Essential versus optional-optimisation information (5.8.4.8, 5.8.4.9). |
| `InbandEventStream` | 0..N | Inband event stream presence (5.10). |
| `Switching` | 0..N | Switch-to times and types (see 5.3.3.4); only allowed if `@timescale` is equal for all Representations of the Adaptation Set and SegmentTimeline is used for durations. |
| `RandomAccess` | 0..N | Random access times and types (5.3.5.5); same two conditions as `Switching`. |
| `GroupLabel`, `Label` | 0..N | Summary label for a group of labels; textual description for annotation/selection (5.3.10). |
| `ProducerReferenceTime` | 0..N | Producer reference time (5.12). |
| `ContentPopularityRate` | 0..N | Popularity level (5.14); mainly for Preselections and Adaptation Sets. |
| `Resync` | 0..N | Resynchronisation point information (5.3.12). |

<a id="c5-3-7-3"></a>

## 5.3.7.3 Type outline

`RepresentationBaseType` = the children above in that order plus any-other-namespace, and the attributes `profiles`, `width`, `height`, `sar`, `frameRate`, `audioSamplingRate`, `mimeType`, `segmentProfiles`, `codecs`, `containerProfiles`,
`maximumSAPPeriod`, `startWithSAP`, `maxPlayoutRate`, `codingDependency`, `scanType`, `selectionPriority` (default 1), `tag`. Schema types: `width`/`height`/`selectionPriority` unsignedInt; `sar` is `RatioType`; `frameRate` is `FrameRateType`;
`maximumSAPPeriod`/`maxPlayoutRate` double; `startWithSAP` is `SAPType`; `scanType` is `VideoScanType` (enumeration `progressive|interlaced|unknown`); `audioSamplingRate` is `AudioSamplingRateType` (list of unsigned ints, length 1 or 2);
`codecs` is `CodecsType` (string matching the RFC 6381 simp-list or fancy-list pattern, built from named pattern entities in print). Ed.6 differences (extra `SegmentSequenceProperties` child): [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md).

## Transcription notes (spec quirks)

- The `Switching` row refers to "subclause 0" (broken cross-reference field, p. 67); the target is 5.3.3.4 by content, not guessed in the verbatim file.
- The `@containerProfiles` row prints "productions ofIETF RFC 6381:2011" with a missing space (p. 65).
- The `@codecs` row cites RFC 6381 "subclause 3.2" and "3.3"; these are RFC sections, not subclauses of this document, even though the verbatim file renders them as links into clause 3 (p. 65).
