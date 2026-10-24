<a id="c3"></a>

# ISO/IEC 23009-1:2022 clause 3 - terms and abbreviations (implementer notes)

Original-wording notes on ISO/IEC 23009-1:2022(E) (5th ed.), clause 3, printed pp. 2-10 (PDF 10-18). Verbatim, page-verified text is in the
private tree: `private/specs-verified/dash/23009-1/03-terms.md`. Term numbers below are the spec's own (3.1.n). See [`INDEX.md`](INDEX.md).

<a id="c3-1"></a>

## 3.1 Glossary of the terms an implementer needs

Clause 3.1 also defers to ISO/IEC 23000-19 for terms not repeated here. Each row is a gloss in our own words, not the spec's definition.

| Term (3.1.n) | Gloss |
|---|---|
| access unit (3.1.1) | One decodable unit of a media stream that has a presentation time. |
| Adaptation Set (3.1.3) | A bundle of encodings of the same content component(s) that a client may swap between. |
| available Segment (3.1.6) | A Segment whose address (plus byte range, if any) currently answers an HTTP GET with a 2xx response. |
| Bitstream Switching Segment (3.1.7) | Optional Segment carrying the data needed to switch into the Representation it belongs to. |
| complementary / dependent Representation (3.1.8, 3.1.12) | A dependent Representation cannot be presented or decoded alone; it needs Segments from its complementary Representations. |
| associated Representation (3.1.5) | A Representation that only adds supplementary or descriptive information to another Representation. |
| earliest presentation time, EPT (3.1.13) | The lowest presentation time among the access units of one stream within a Media Segment or Subsegment. |
| group (3.1.16) | A set of Adaptation Sets that are not meant to be played at the same time. |
| HTTP-URL (3.1.17), data URL (3.1.11) | URL restricted to the http/https scheme, respectively to the `data` scheme. |
| Index Segment (3.1.18) | A Segment that mainly holds indexing data for Media Segments. |
| Initialization Segment (3.1.19) | A Segment with the metadata needed before any Media Segment of the Representation can be presented. |
| Media Segment (3.1.28) | A Segment in the media format in use that plays when combined with its Initialization Segment (if any) and zero or more earlier Segments. |
| Media Subsegment (3.1.30) | A Subsegment containing only media data, no Segment Index. |
| Media Presentation (3.1.25), MPD (3.1.26) | The whole (bounded or unbounded) presentation, and the document that describes it for a streaming service. |
| Media Presentation timeline (3.1.27) | The Periods' timelines concatenated; shared by every Representation of a Period. |
| MPD start time / MPD duration (3.1.32, 3.1.33) | The approximate start and length of a Media Segment as signalled in the MPD (the true values are in the Segment). |
| Period (3.1.34) | A time interval of the Media Presentation; consecutive Periods tile the presentation. |
| Preselection / Main Adaptation Set (3.1.35, 3.1.20) | A set of components meant to be consumed together; the Main Adaptation Set is the member holding the Initialization Segment for the complete experience. |
| presentation time (3.1.36) | The time that places an access unit on the Media Presentation timeline. |
| remote element entity (3.1.37) | Elements fetched from an `@xlink:href` HTTP-URL (XLink "remote resource"). |
| Representation (3.1.38) | One deliverable encoding of one or more media streams, with its descriptive metadata. |
| Resynchronization Point (3.1.39) | A position inside a Segment from which processing of the Representation can start. |
| Segment (3.1.40) | The unit addressed by an HTTP-URL (optionally with a byte range) in the MPD, or by a data URL. |
| Segment availability start time (3.1.41) / adjusted (3.1.42) / end time (3.1.43) | Wall-clock instants at which a Segment becomes, respectively ceases to be, an available Segment; "adjusted" is the instant after the offset adjustments of 5.3.9.5.3. |
| Segment Index (3.1.44), Subsegment (3.1.51) | A compact time-range to byte-range map inside a Media Segment, and the unit it indexes. |
| Segment Sequence / Segment Track (3.1.45, 3.1.46) | Segments sharing a common address prefix; concatenated Segments forming a track with possible conformance properties. |
| stream access point, SAP (3.1.48) | A position from which playback can begin using only data from there onward plus any initialization data. |
| Sub-Representation (3.1.50) | A part of a Representation described in the MPD that exists for the entire Period. |
| valid Segment URL (3.1.52) | An HTTP-URL promised to reference a Segment throughout its availability period. |
| wall-clock time (3.1.53) | UTC. |

Other terms in 3.1 (accessibility 3.1.2, asset 3.1.4, continuous media 3.1.9, DASH metric 3.1.10, event/event stream/message 3.1.14, 3.1.15, 3.1.31,
media content / component / component type / period 3.1.21-3.1.24, media stream 3.1.29, spatial object 3.1.47, sub-asset 3.1.49) are general vocabulary;
read them verbatim in the private file when exact wording matters.

<a id="c3-2"></a>

## 3.2 Abbreviations

About 70 abbreviations are listed (printed pp. 6-8). Those that matter for MPD and Segment work: DASH, MPD, SAP, EPT, MBT (minimum buffer time),
ISO BMFF, CMAF, 4CC, SRD (spatial relationship description), HDCP, URN/URL/URI, UTC, NTP/SNTP, TS/PES/PCR (MPEG-2 transport), SPS/SEI/IDR/GDR
(codec terms), and VAST (ad serving). The full table is verbatim in the private file.

<a id="c3-3"></a>

## 3.3 Notation conventions of the document

- Element names start with an upper-case letter; `Parent.Child` names containment; attributes start lower-case and are written `@attr`, or
  `Element@attr` to point at one element's attribute. Multi-word names are camel-cased.
- Prefix `namespace:Element` / `@namespace:attr` marks extension namespaces; extension specifications are expected to document such names in a
  semantics table with their prefix.
- Variables defined by the spec are italicised; structure names of the data model (Period, Adaptation Set, Representation, Segment) are capitalised.
- "This clause" means everything under the same first-level number; "this subclause" means the lowest-level heading only.
- ABNF (RFC 5234) is used to pin down attribute/element value syntaxes that XML Schema leaves loose.

## Transcription notes (spec quirks)

- Definitions 3.1.13 and 3.1.30 refer to "Subsegment (3.1.50)", but 3.1.50 is Sub-Representation; Subsegment is 3.1.51 (printed pp. 3-4).
- Definition 3.1.34 (Period) refers to "Media Presentation (3.1.24)"; Media Presentation is 3.1.25 and 3.1.24 is media content period (p. 4).
- Capitalisation is inconsistent inside some definitions (3.1.13, 3.1.20) and 3.1.8 does not bold its term; cosmetic only.
