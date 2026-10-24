---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 3 - Terms, definitions, symbols and abbreviated terms"
pdf_pages: "10-18"
printed_pages: "2-10"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c3"></a>

# 3 Terms, definitions, symbols and abbreviated terms

<a id="c3-1"></a>

## 3.1 Terms and definitions

For the purposes of this document, the terms and definitions given in ISO/IEC 23000-19 and the following apply.

ISO and IEC maintain terminology databases for use in standardization at the following addresses:

- ISO Online browsing platform: available at https://www.iso.org/obp
- IEC Electropedia: available at https://www.electropedia.org/

**3.1.1 access unit**  
unit of a media stream (3.1.29) with an assigned Media Presentation time

**3.1.2 accessibility**  
degree to which a media content or certain media content components (3.1.22) are available to as many people as possible

**3.1.3 Adaptation Set**  
set of interchangeable encoded versions of one or several media content components (3.1.22)

**3.1.4 asset**  
content including media and metadata together with the rights to use the content by the content provider

<!--PB printed 2-->

**3.1.5 associated Representation**  
Representation (3.1.38) which provides supplemental or descriptive information for at least one other Representation

**3.1.6 available Segment**  
Segment (3.1.40) that is accessible at its assigned HTTP-URL (3.1.17) and a possibly assigned byte range that is the request with an HTTP GET results in a reply of the Segment and 2xx status code

**3.1.7 Bitstream Switching Segment**  
Segment (3.1.40) that if present contains essential data to switch to the Representation (3.1.38) it is assigned to

**3.1.8 complementary Representation**  
Representation (3.1.38) which complements at least one dependent Representation (3.1.12)

**3.1.9 continuous media**  
media with an inherent notion of time

EXAMPLES Speech, audio, video, timed text or timed metadata.

**3.1.10 DASH metric**  
metric computed by the DASH Client and uniquely identified by a key

**3.1.11 data URL**  
URL with a fixed scheme “data”

**3.1.12 dependent Representation**  
Representation (3.1.38) for which Segments (3.1.40) from its complementary Representations (3.1.8) are necessary for presentation and/or decoding of the contained media content components (3.1.22)

**3.1.13 earliest presentation time**  
smallest presentation time (3.1.36) of any access unit (3.1.1) of a Media Segment (3.1.28) Or Subsegment (3.1.50) for a media stream (3.1.29)

**3.1.14 event**  
aperiodic sparse media-time related auxiliary information to the DASH Client or to an application

**3.1.15 event stream**  
sequence of related events (3.1.14)

**3.1.16 group**  
collection of Adaptation Sets (3.1.3) that are not expected to be presented simultaneously

<!--PB printed 3-->

**3.1.17 HTTP-URL**  
URL with a fixed scheme of “http” or “https”

**3.1.18 Index Segment**  
Segment (3.1.40) that primarily contains indexing information for Media Segments (3.1.28)

**3.1.19 Initialization Segment**  
Segment (3.1.40) containing metadata that is necessary to present the media streams (3.1.29) encapsulated in Media Segments (3.1.28)

**3.1.20 Main Adaptation Set**  
Adaptation Set (3.1.3) In A Preselection (3.1.35) that contains the Initialization Segment (3.1.19) for the complete experience

**3.1.21 media content**  
single media content period (3.1.24) or contiguous sequence of media content periods

**3.1.22 media content component**  
single continuous component of the media content (3.1.21) with an assigned media content component type (3.1.23)

**3.1.23 media content component type**  
single type of media content (3.1.21)

EXAMPLES Audio, video, or text.

**3.1.24 media content period**  
set of media content components (3.1.22) that have a common timeline as well as relationships on how they can be presented

**3.1.25 Media Presentation**  
collection of data that establishes a bounded or unbounded presentation of media content (3.1.21)

**3.1.26 Media Presentation Description / MPD**  
formalized description for a Media Presentation (3.1.25) for the purpose of providing a streaming service

**3.1.27 Media Presentation timeline**  
concatenation of the timeline of all Periods (3.1.34) which itself is common to all Representations (3.1.38) in the Period

<!--PB printed 4-->

**3.1.28 Media Segment**  
Segment (3.1.40) that complies with media format in use and enables playback when combined with zero or more preceding Segments and an Initialization Segment (3.1.19) (if any)

**3.1.29 media stream**  
encoded version of a media content component (3.1.22)

**3.1.30 Media Subsegment**  
Subsegment (3.1.50) that only contains media data but no Segment Index (3.1.44)

**3.1.31 message**  
part of an event (3.1.14) containing information that is exclusively handled by the event handler

**3.1.32 MPD start time**  
approximate presentation start time of a Media Segment (3.1.28) signalled in MPD (3.1.26)

**3.1.33 MPD duration**  
approximate presentation duration of a Media Segment (3.1.28) signalled in MPD (3.1.26)

**3.1.34 Period**  
interval of the Media Presentation (3.1.24), where a contiguous sequence of all Periods constitutes the Media Presentation

**3.1.35 Preselection**  
set of media content components (3.1.22) that are intended to be consumed jointly

**3.1.36 presentation time**  
time associated to an access unit (3.1.1) that maps it to the Media Presentation timeline (3.1.27)

**3.1.37 remote element entity**  
entity that contains one or more elements and is referenced in the MPD (3.1.26) with an HTTP-URL (3.1.17) contained in an @xlink:href attribute, referred to as "remote resource" by XLink

**3.1.38 Representation**  
collection and encapsulation of one or more media streams (3.1.29) in a delivery format and associated with descriptive metadata

**3.1.39 Resynchronization Point**  
point within a Segment from which it is possible to start processing the Representation

<!--PB printed 5-->

**3.1.40 Segment**  
unit of data associated with an HTTP-URL (3.1.17) and optionally a byte range that are specified by an MPD (3.1.26), or with a data URL (3.1.11)

**3.1.41 Segment availability start time**  
latest time instant in wall-clock time (3.1.53) at which a Segment (3.1.40) becomes an available Segment (3.1.6)

**3.1.42 adjusted Segment availability start time**  
time instant in wall-clock time (3.1.53) at which a Segment (3.1.40) becomes an available Segment (3.1.6)

**3.1.43 Segment availability end time**  
time instant in wall-clock time (3.1.53) at which a Segment (3.1.40) ceases to be an available Segment (3.1.6)

**3.1.44 Segment Index**  
compact index of the time range to byte range mapping within a Media Segment (3.1.28) separately from the MPD (3.1.26)

**3.1.45 Segment Sequence**  
sequence of Segments (3.1.40) that are sharing a common address prefix

**3.1.46 Segment Track**  
concatenation of Segments (3.1.40) forming a track with potential conformance properties

**3.1.47 spatial object**  
media content component (3.1.22) corresponding to a region in a coordinate system associated to this media content component

**3.1.48 stream access point / SAP**  
position in a Representation (3.1.38) enabling playback of a media stream (3.1.29) to be started using only the information contained in Representation data starting from that position onwards preceded by initializing data in the Initialization Segment (3.1.19), if any

**3.1.49 sub-asset**  
media content component (3.1.22) (or part thereof) identified as corresponding to a part of an asset (3.1.4)

<!--PB printed 6-->

**3.1.50 Sub-Representation**  
part of a Representation (3.1.38) described in the MPD (3.1.26) that is present in the entire Period (3.1.34)

**3.1.51 Subsegment**  
unit within Media Segments (3.1.28) that is indexed by a Segment Index (3.1.44)

**3.1.52 valid Segment URL**  
HTTP-URL (3.1.17) that is promised to reference a Segment (3.1.40) during its Segment availability period

**3.1.53 wall-clock time**  
time as stated by UTC

<a id="c3-2"></a>

## 3.2 Symbols and abbreviated terms



| Abbreviation | Term |
|---|---|
| 4CC | four character code |
| ABNF | augmented Backus-Naur form |
| ACK | acknowledgement message in TCP |
| AEMS | application event or metadata streams |
| API | application programming interface |
| AVC | advanced video coding |
| CAT | conditional access table |
| CDN | content delivery network |
| CMAF | common media application format |
| DASH | dynamic adaptive streaming over HTTP |
| DM | DASH metrics |
| DOM | document object model |
| DRM | digital rights management |
| ECM | entitlement control message |
| EIDR | entertainment identifier registry |
| EME | encrypted media extensions |
| EMIO | event/metadata internal object |
| EMM | entitlement management message |
| EPT | earliest presentation time |
| GAA | generic authentication architecture |
| GBA | generic bootstrapping architecture |
| GDR | gradual decoding refresh |
| GMT | Greenwich Mean Time |
| <!--PB printed 7--> GOP | group of pictures |
| GPS | global positioning system |
| HDCP | high-bandwidth digital content protection |
| HDMI | high definition multimedia interface |
| HEVC | high efficiency video coding |
| HTML | hypertext markup language |
| HTTP | hypertext transfer protocol |
| HTTPS | secure hypertext transfer protocol |
| IAB | interactive advertising bureau |
| IDR | instantaneous decoding refresh |
| IP | internet protocol |
| ISO BMFF | ISO base media file format |
| LAT | Latest Arrival Time |
| MBT | minimum buffer time |
| MIME | multipurpose internet mail extensions |
| MPD | media presentation description |
| MPP | MPD patch |
| MVC | multi-view video coding |
| NGA | next generation audio |
| NTP | network time protocol |
| OATC | open authentication technical committee |
| OAuth | open standard for authorization |
| OMAP | online multimedia authorization protocol |
| PAT | program association table |
| PCR | program clock reference |
| PES | packetized elementary stream |
| PID | packet identifier |
| PMT | program map table |
| PSI | program specific information |
| PTS | presentation time stamp |
| RAP | random access point |
| SAML | security assertion markup language |
| SAP | stream access point |
| SEI | supplementary enhancement information |
| SNTP | simple NTP |
| SPS | sequence parameter set |
| <!--PB printed 8--> SRD | spatial relationship description |
| SVC | scalable video coding |
| TCP | transmission control protocol |
| TLS | transport layer security |
| TS | transport stream |
| URI | uniform resource identifier |
| URL | uniform resource locator |
| URN | uniform resource name |
| UTC | coordinated universal time |
| UUID | universally unique identifier |
| VAST | video ad serving template |
| VOD | video-on-demand |
| XML | extensible mark-up language |

<a id="c3-3"></a>

## 3.3 Conventions

The following naming conventions apply in this document.

- Elements in an XML document are identified by an upper-case first letter and in bold face as Element. To express that an element Element1 is contained in another element Element2, the following format is used: Element2.Element1. If an element's name consists of two or more combined words, camel-casing is typically used, e.g. ImportantElement. Elements may be present either exactly once, or the minimum and maximum occurrence is defined by <minOccurs>... <maxOccurs>.
- Attributes in an XML document are identified by a lower-case first letter as well as they are preceded by an '@'-sign, e.g. @attribute. To point to a specific attribute @attribute contained in an element Element, one may write Element@attribute. If an attribute's name consists of two or more combined words, camel-casing is typically used after the first word, e.g. @veryImportantAttribute. Attributes may have assigned a status in the XML as mandatory (M), optional (O), optional with default value (OD) and conditionally mandatory (CM).
- Namespace qualification of elements and attributes is used as per XML standards, in the form of namespace:Element or @namespace:attribute. The fully qualified namespace is provided in the schema fragment associated with the declaration. External specifications extending the namespace of DASH are expected to document the element name in the semantic table with an extension namespace prefix.
- Variables defined in the context of this document are specifically highlighted with italics, e.g. InternalVariable.
- Structures that are defined as part of the hierarchical data model are identified by an upper-case first letter, e.g. Period, Adaptation Set, Representation, Segment, etc.
- The term "this clause" refers to the entire clause included within the same first heading number. The term "this subclause" refers to all text contained in the subclause with the lowest hierarchy heading.

<!--PB printed 9-->

- For improved interoperability, this document uses ABNF notation according to IETF RFC 5234 to specify certain underdefined values of XML attributes and elements.

## Transcription notes

> **Spec note:** 3.1.13 and 3.1.30 cite "Subsegment (3.1.50)", but 3.1.50 is Sub-Representation; Subsegment is 3.1.51. Reproduced as printed.
>
> **Spec note:** 3.1.34 cites "Media Presentation (3.1.24)"; Media Presentation is 3.1.25 (3.1.24 is media content period). Reproduced as printed.
>
> **Spec note:** Capitalisation quirks reproduced as printed: 3.1.13 "Or Subsegment"; 3.1.20 "In A Preselection". In 3.1.8 the term "complementary Representation" is not set in bold in the PDF.
>
> **Spec note:** Typographic emphasis (bold term names, italic cross-referenced terms) is reproduced only for term names; italic cross-referenced terms are not marked up (see VERIFICATION.md, formatting policy).
