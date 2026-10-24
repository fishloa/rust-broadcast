---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 4 - Overview"
pdf_pages: "18-29"
printed_pages: "10-21"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c4"></a>

# 4 Overview

<a id="c4-1"></a>

## 4.1 System description

Dynamic adaptive streaming over HTTP (DASH) specifies XML and binary formats that enable delivery of continuous media content from standard HTTP servers to HTTP clients and enable caching of content by standard HTTP caches.

This document primarily defines two formats:

- The Media Presentation Description (MPD) describes a Media Presentation, i.e. a bounded or unbounded presentation of continuous media content. In particular, it defines formats to announce resource identifiers for Segments and to provide the context for these identified resources within a Media Presentation. These resource identifiers are HTTP-URLs possibly combined with a byte range, or with a data URL.
- The Segment formats specify the formats of the entity body of the HTTP response to an HTTP GET request or a partial HTTP GET with the indicated byte range using HTTP/1.1 as defined in IETF RFC 7233 to a resource identified in the MPD. Segments typically contain efficiently coded media data and metadata conforming to or at least closely aligned with common media formats.

The MPD provides sufficient information for a client to provide a streaming service to the user by accessing the Segments through the protocol specified in the scheme of the defined resources. In the context of this document, the assumed protocol is HTTP/1.1. Such a client is referred to as a DASH Client in the remainder of this document. However, this document does not provide a normative specification for such a client.

Figure 1 shows a possible deployment architecture in which the formats defined in this document may be used. Boxes with solid lines indicate devices that are mentioned in this document as they host or process the formats defined in this document whereas dashed boxes are conceptual or transparent. This document deals with the definition of formats that are accessible on the interface to the DASH Client, indicated by the solid lines. Any other formats or interfaces are outside the scope of this document. In the considered deployment scenario, it is assumed that the DASH Client has access to an MPD. The MPD provides sufficient information for the DASH Client to provide a streaming service to the user by requesting Segments from an HTTP server and demultiplexing, decoding and rendering the included media streams.

<!--PB printed 10-->

```text
 [redraw of Figure 1, PDF p.19 (printed 11) - dashed boxes are conceptual/transparent, solid boxes are devices]

  (dashed) DASH Media         <- - - ->  (dashed) MPD Delivery Function
  Presentation Preparation   (arrow from Preparation up to MPD Delivery Function)
        ^ - - - - - - - - - ->  (solid) DASH Segment Delivery Function (HTTP Server)

  MPD Delivery Function - - - "MPD" (passes dashed through the HTTP Cache box) - - - -> (solid) DASH Client
  DASH Segment Delivery Function <---- "Segments" (through (dashed, grey) HTTP Cache) ----> DASH Client
```

**Figure 1 — Example system for DASH formats**

Although the formats are initially designed to be used in the above deployment scenario, their application is obviously not restricted to this scenario. The particular aspect on "HTTP" in DASH is the usage of HTTP-URLs in the MPD for the purpose to refer to Segments. The usage of HTTP-URLs enables unique location information and it provides well-defined methods to access the resources, in particular HTTP GET and HTTP partial GET.

<a id="c4-2"></a>

## 4.2 DASH Client model

The design of the formats defined in this document is based on the informative client model as shown in Figure 2. The figure illustrates the logical components of a conceptual DASH Client model and the relation to other components in a media streaming application. In this figure, the DASH access engine receives the Media Presentation Description (MPD), constructs and issues requests and receives Segments or parts of Segments. The DASH Client may use metadata provided in the MPD for the selection of media components by communication with the media streaming application. Such metadata may for example include codec capability information, language codes, accessibility information and other information for the selection of media components. In the context of this document, the output of the DASH access engine consists of media in MPEG container formats (ISO/IEC 14496-12 ISO base media file format or ISO/IEC 13818-1 MPEG-2 Transport Stream), or parts thereof, together with timing information that maps the internal timing of the continuous media to the timeline of the Media Presentation. In Annex F, guidance on enabling the use of this document with other container formats is provided. In addition, the DASH access engine may also receive and extract Events that are related to the media time. The events may be processed in the DASH Client or may be forwarded to an event processing application in the execution environment of the DASH Client.

<!--PB printed 11-->

```text
 [redraw of Figure 2, PDF p.20 (printed 12); everything inside the box "Media Streaming Application"]

   Selection Logic <--- "MPD Selection metadata" --- DASH access engine
   Selection Logic ---- "Selected Adaptation Sets" --> DASH access engine

   MPD (dashed arrow)     --> DASH access engine
   Segment data (solid)   --> DASH access engine
   DASH access engine --- "MPEG Format media + timing" --> Media engine --> Media output
   DASH access engine --- "Event + timing"             --> Media engine (second box)
```

**Figure 2 — DASH Client model**

Furthermore, Annex K defines the Service Description to support and guide DASH Client operations.

<a id="c4-3"></a>

## 4.3 DASH data model overview

DASH is intended to support a media-streaming model for delivery of continuous media content in which control lies primarily with the client. Clients may request data using the HTTP protocol from standard web servers that have no DASH-specific capabilities. Consequently, this document focuses not on client or server procedures but on the data formats used to provide a DASH Media Presentation.

The collection of encoded and deliverable versions of continuous media content and the appropriate description of these form a Media Presentation. Media content is composed of a single or multiple contiguous media content **periods** in time. Content in different media content periods may be completely independent or certain periods of a Media Presentation may belong to the same Asset, for example a Media Presentation is a collection of main program composed of multiple periods, each assigned to the same Asset, and interleaved with inserted advertisement periods. Each media content period is composed of one or multiple **media content components,** for example audio components in various languages, different video components providing different views of the same program, subtitles in different languages, etc. Each media content component has an assigned **media content component type**, for example audio or video. The same asset over multiple periods may be identified by a DASH descriptor enabling DASH Clients to maintain the continuity across periods' boundaries. Furthermore, sub-assets composing the same asset may also be identified using a similar method. For instance, if an asset is composed of multiple video components, sub-assets enable selecting the previously selected video component after an ad insertion.

Each media content component may have several encoded versions, referred to as **media streams**. Each media stream inherits the properties of the media content, the media content period, the media content component from which it was encoded and, in addition, it gets assigned the properties of the encoding process such as sub-sampling, codec parameters, encoding bitrate, etc. This descriptive metadata is relevant for static and dynamic selection of media content components and media streams.

<!--PB printed 12-->

```text
 [redraw of Figure 3, PDF p.21 (printed 13); nesting of boxes; axes: horizontal arrow "Time", vertical arrow "Choice"]

 Media Presentation
 +- Period  (first, wide box)                              +- Period (second, narrow box) ...
 |   +- Preselection  (dashed box)
 |   |   +- Adaptation Set
 |   |   |   +- Representation
 |   |   |   |     Initialization Segment | Media Segment ···· Media Segment
 |   |   |   |     Self-Initializing Segment: Sub-Segment ···· Sub-Segment
 |   |   |   |     Sub-Representation (dashed box)
 |   |   |   +- ······· Representation
 |   |   +- ······· Adaptation Set
 |   +- ······· Preselection (dashed box)
 +- ·······  further Periods along the Time axis
```

**Figure 3 — DASH High-Level Data Model**

DASH is based on a hierarchical data model aligned with the presentation in Figure 3. In the horizontal domain, this shows the sequence in time of the Media Presentation, and in the vertical domain it shows the choices offered in a Media Presentation, to be selected by the DASH Client in a static and dynamic manner. A DASH **Media Presentation** is described by a Media Presentation Description document. This describes the sequence of **Periods** (see subclause5.3.2) in time that make up the Media Presentation. A Period typically represents a media content period during which a consistent set of encoded versions of the media content is available i.e. the set of available bitrates, languages, captions, subtitles etc. does not change during a Period.

Within a Period, material is arranged into **Adaptation Sets** (see subclause [5.3.3](05-3-3-adaptation-set.md#c5-3-3)). An Adaptation Set represents a set of interchangeable encoded versions of one or several media content components (see subclause [5.3.4](05-3-4-content-component.md#c5-3-4)). For example, there may be one Adaptation Set for the main video component and a separate one for the main audio component. If there is other material available, for example captions or audio descriptions, then these may each have a separate Adaptation Set. Material may also be provided in multiplexed form, in which case interchangeable versions of the *multiplex* may be described as a single Adaptation Set, for example an Adaptation Set containing both the main audio and main video for a Period. Each of the multiplexed components may be described individually by a media content component description. In the third edition, the concept of **Preselections** (see subclause 5.3.11) was added in order to enable the combination of different Adaptation Sets into a single decoding instance and user experience.

<!--PB printed 13-->

An Adaptation Set contains a set of **Representations** (see subclause [5.3.5](05-3-5-representation.md#c5-3-5)). A Representation describes a *deliverable encoded version* of one or several media content components. A Representation includes one or more media streams (one for each media content component in the multiplex). Any single Representation within an Adaptation Set is sufficient to render the contained media content components. By collecting different Representations in *one* Adaptation Set, the Media Presentation author expresses that the Representations represent perceptually equivalent content. Typically, this means that clients may switch dynamically from Representation to Representation within an Adaptation Set in order to adapt to network conditions or other factors. Switching refers to the presentation of decoded data up to a certain time t, and presentation of decoded data of another Representation from time t onwards. If Representations are included in one Adaptation Set, and the client switches properly, the Media Presentation is expected to be perceived seamless across the switch. DASH Clients may ignore Representations containing unsupported codecs or rendering technologies, or that are otherwise unsuitable.

Within a Representation, the content may be divided in time into **Segments** (see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9) and Clause 5.15) for proper accessibility and delivery. In order to access a Segment, a URL is provided for each Segment. Consequently, a Segment is the largest unit of data that can be retrieved with a single HTTP request. For segmented Representations, two types of Segments are differentiated: **Initialization Segments** contain static metadata for the Representation, **Media Segments** contain media samples and advance the timeline. Representations may also be organized by a single self-initialitizing Segment which contains both initialization information as well as media data.

NOTE The MPD can also include a byte range with the URL, meaning that the Segment is contained in the provided byte range of some larger resource. An intelligent client can in principle construct a single request for multiple Segments, but this is not the typical case.

DASH defines different timelines. One of the key features in DASH is that encoded versions of different media content components share a common timeline. The presentation time of each access unit within the media content is mapped to the global common presentation timeline for synchronization of different media components and to enable seamless switching of different coded versions of the same media components. This timeline is referred to as Media Presentation timeline. The Media Segments themselves contain accurate Media Presentation timing information enabling synchronization of components and seamless switching.

A second timeline is used to signal to clients the availability time of Segments at the specified HTTP-URLs. These times are referred to as **Segment availability times** and are provided in wall-clock time. Clients typically compare the wall-clock time to Segment availability times before accessing the Segments at the specified HTTP-URLs in order to avoid erroneous HTTP request responses. For static Media Presentations, the availability times of all Segments are identical. For dynamic Media Presentations, the availability times of segments depend on the position of the Segment in the Media Presentation timeline, i.e. the Segments get available over time. Whereas static Media Presentations are suitable to offer On-Demand content, dynamic Media Presentations are mostly suitable to offer live services.

Segments are assigned a duration, which is the duration of the media contained in the Segment when presented at normal speed. Typically, all Segments in a Representation have the same or a roughly similar duration. However, Segment duration may differ from Representation to Representation. A DASH presentation can be constructed with relative short segments (for example a few seconds), or longer Segments including a single Segment for the whole Representation.

Short Segments are usually required in the case of live content, where there are restrictions on end-to-end latency. The duration of a Segment is typically a lower bound on the end-to-end latency. DASH does not support the possibility for Segments to be extended over time: a Segment is treated as an object as a complete and discrete unit that is made available in its entirety. However, this does not prevent from applying advanced HTTP transfer modes such as chunked transfer to optimize deployments and reduce end-to-end latency.

<!--PB printed 14-->

In particular, self-initializing Segments may be further subdivided into **Subsegments** each of which contains a number of complete access units. There may also be media-format-specific restrictions on Subsegment boundaries; for example, in the ISO base media file format, a Subsegment contains a number of complete movie fragments. If a Segment is divided into Subsegments, they are described by a compact **Segment index**, which provides the presentation time range in the Representation and corresponding byte range in the Segment occupied by each Subsegment. Clients may download this index in advance and then issue requests for individual Subsegments.

Clients may switch from Representation to Representation within an Adaptation Set at any point in the media. However, switching at arbitrary positions can be complex because of coding dependencies within Representations and other factors, potentially requiring parallel download and decoding in the DASH Client. It is also desirable to avoid downloading 'overlapping' data i.e. media for the same time period from multiple Representations. Usually, switching is simplest at a stream access point (SAP) in the new stream. In order to formalize requirements related to switching, DASH defines a codec-independent concept of stream access points and identifies various types of stream access points.

Segmentation and Subsegmentation may be performed in ways that make switching simpler. For example, in the very simplest cases, each Segment or Subsegment begins with a SAP and the boundaries of Segments or Subsegments are aligned across the Representations of one Adaptation Set. In this case, switching Representation involves playing to the end of a (Sub)Segment of one Representation and then playing from the beginning of the next (Sub)Segment of the new Representation. The Media Presentation Description and Segment Index provide various indications, which describe properties of the Representations that may make switching simpler. Profiles of this document can then require these indicators to be set in certain ways, making implementation of clients for those profiles simpler at the cost of requiring the media data to obey the indicated constraints.

For On-Demand services, the Media Presentation Description is typically a static document describing the various aspects of the Media Presentation. All Segments of the Media Presentation are available on the server once any Segment is available. For live services, however, Segments become available with time as the content is produced and therefore, dynamic Media Presentations are suitable. The Media Presentation Description may be updated regularly to reflect changes in the presentation over time, for example Segment URLs for new segments may be added to the MPD and those for old, no longer available Segments may be removed. However, if Segment URLs are described using a template, this updating may not be necessary except for some redundancy/failover cases.

Events may be provided in the MPD or within a Representation in order to signal aperiodic information to the DASH Client or to an application. Events are timed, i.e. each event starts at a specific media presentation time and typically has a duration. Events include DASH-specific signalling or application-specific events. Examples for events are indication of MPD updates on the server, possibly providing the detailed update as part of the messages. The event mechanisms may also be used to deliver media time related application events, for example information about ad insertion opportunities, etc.

<a id="c4-4"></a>

## 4.4 Protocols

This document may be deployed in a system according Figure 1 for which

- the DASH Client includes a *client* as specified in IETF RFC 7230<sup>[22]</sup>, IETF RFC 7231, IETF RFC 7232<sup>[23]</sup>, IETF RFC 7233, IETF RFC 7234<sup>[24]</sup> and IETF RFC 7235<sup>[25]</sup>, and
- the HTTP Server hosting the DASH Segments complies with a *server* as specified in IETF RFC 7230, IETF RFC 7231, IETF RFC 7232, IETF RFC 7233, IETF RFC 7234 and IETF RFC 7235.

DASH Clients typically use the HTTP GET method or the HTTP partial GET method, as specified in IETF RFC 7231:2014, subclause [4.3](#c4-3), to access Segments or parts thereof.

The use of HTTP as a transport protocol inherently provides many advanced features such as caching (IETF RFC 7233), redirection (IETF RFC 7231) or authentication (IETF RFC 7235). As another example, transport security in HTTP-based delivery may be achieved by using HTTP over TLS as specified in IETF RFC 2818. Yet another example is the use of HTTP state management mechanisms (also known as Cookies) as defined in IETF RFC 6265<sup>[21]</sup>.

<!--PB printed 15-->

However, the formats defined in this document may also be used with other protocols. In particular, the objects may be delivered with any object delivery protocol that provides a binding between an HTTP-URL and the delivered object.

<a id="c4-5"></a>

## 4.5 Media stream and Representation properties

<a id="c4-5-1"></a>

### 4.5.1 Switching and Random Access Support

The formats defined in this document are designed to provide a good user experience even in cases where the access bandwidth between the DASH segment delivery function or the cache varies. A key functionality is the ability of the DASH Client to seamlessly switch across different Representations of the same media component without severely impacting the user experience.

Assume two Representations A and B. A switch from Representation A to Representation B at media time t is considered seamless, if the result of the presentation after this switch is applied is the same as if Representation A was decoded from the beginning and presented up to time t and Representation B is decoded from the beginning and presented from time t onwards.

Media Presentations may provide different Representations in one Adaptation Set representing the same media component. If such Representations are properly time-aligned (as expected by the Media Presentation), then DASH Clients may apply seamless switching across different Representations provided in one Adaptation Set at any time t to obtain a perceptually continuous experience.

However, in practical implementations, the operation of seamless switching can be complex, as switching at time t can require parallel download and decoding of two Representations. Therefore, providing suitable switching opportunities in regular time intervals simplifies client implementations. This document provides means for providing suitable switching opportunities and in addition provides abilities to signal the position and media time of the switching opportunities.

For this purpose, this subclause defines three relevant concepts to support seamless switching:

- Media stream access points in subclause [4.5.2](#c4-5-2) to signal positions where to easily switch to a Representation, and in addition where to suitable access a Representation at start-up or seek.
- Non-overlapping Segments and Subsegments in subclause [4.5.3](#c4-5-3) to signal that, at the signalled stream access points, no overlap decoding of Representations is necessary in order to provide a continuous switch.
- Segment concatenation in subclause [4.5.4](#c4-5-4) to signal that the concatenation of two Representations at a switch point results in a conforming bitstream.

These three properties are neither sufficient nor necessary for seamless switching, but certain implementation or profiles may use these properties in order to simplify practical implementations.

<a id="c4-5-2"></a>

### 4.5.2 Media stream access points

To be able to access a Representation, each of the media streams that are contained in the Representation requires media stream access points (SAPs). SAPs in the context of this document refer to the SAP definition in ISO/IEC 14496-12. ISO/IEC 14496-12 SAP types defines different types of SAPs that provide a relationship between the position where a stream can be accessed, relative to the start of a Segment or Subsegment, its presentation time and the presentation times and position of other access unit in the stream. The same SAP type definitions shall apply for this document.

<!--PB printed 16-->

A SAP is a position in a Representation that enables playback of a media stream to be started using only the information contained in Representation data starting from that position onwards (preceded by initializing data in the Initialization Segment, if any).

For each SAP, the properties, I<sub>SAP</sub>, T<sub>SAP</sub>, I<sub>SAU</sub>, T<sub>DEC</sub>, T<sub>EPT</sub>, and T<sub>PTF</sub>, are identified and defined in ISO/IEC 14496-12 SAP properties.

In particular, T<sub>SAP</sub> is defined to be the earliest presentation time of any access unit of the media stream such that all access units of the media stream with presentation time greater than or equal to T<sub>SAP</sub> can be correctly decoded using data in the Representation starting at byte position I<sub>SAP</sub> and no data before I<sub>SAP</sub>.

NOTE The type of SAP is dependent only on which access units are correctly decodable and their arrangement in presentation order. The types informally correspond with some common terms:

- Type 1 corresponds to what is known in some coding schemes as a “Closed GoP random access point” (in which all access units, in decoding order, starting from I<sub>SAP</sub> can be correctly decoded, resulting in a continuous time sequence of correctly decoded access units with no gaps) and in addition the access unit in decoding order is also the first access unit in presentation order.
- Type 2 corresponds to what is known in some coding schemes as a “Closed GoP random access point”, for which the first access unit in decoding order in the media stream starting from I<sub>SAU</sub> is not the first access unit in presentation order.
- Type 3 corresponds to what is known in some coding schemes as an “Open GoP random access point”, in which there are some access units in decoding order following I<sub>SAU</sub> that cannot be correctly decoded and have presentation times less than T<sub>SAP</sub>.
- Type 4 corresponds to what is known in some coding schemes as a "gradual decoder refresh (GDR) random access point”, in which there are some access units in decoding order starting from and following I<sub>SAU</sub> that cannot be correctly decoded and have presentation times less than T<sub>SAP</sub>.
- Type 5 corresponds to the case for which there is at least one access unit in decoding order starting from I<sub>SAP</sub> that cannot be correctly decoded and has presentation time greater than T<sub>DEC</sub> and where T<sub>DEC</sub> is the earliest presentation time of any access unit starting from I<sub>SAU</sub>.
- Type 6 corresponds to the case for which there is at least one access unit in decoding order starting from I<sub>SAP</sub> that cannot be correctly decoded and has presentation time greater than T<sub>DEC</sub> and where T<sub>DEC</sub> is not the earliest presentation time of any access unit starting from I<sub>SAU</sub>.

SAPs are mostly relevant for two purposes in this document:

1) For randomly accessing a Media Presentation, for example at the startup of the Media Presentation, after a seeking operation or after an error event especially in live cases.

2) To permit switching between two Representations whereby for seamless switching each media stream *i* in the switch-from Representation is presented up to T<sub>SAP</sub>(*i*) and each media stream *i* in the switch-to Representation is presented from the media stream access point starting from T<sub>SAP</sub>(*i*).

There are obvious benefits for the client to be able to identify SAPs and one or several of their properties, in particular I<sub>SAP</sub> and T<sub>SAP</sub> for each media stream without requiring to access data at positions following I<sub>SAP</sub>. DASH provides functionalities to explicitly signal such information by using signals in the MPD or the Segment Index or combinations of the two.

<a id="c4-5-3"></a>

### 4.5.3 Non-overlapping Segments and Subsegments

Segments and Subsegments represent units for which the client has an exact map on how to access and download the unit using HTTP GET or HTTP partial GET methods.

Segments (respectively Subsegments) are typically generated by segmenting encoded media streams into appropriate units. If the generation of Segments (respectively Subsegments) adheres to certain rules, then the sequential decoding and presentation of Media Segments (respectively Subsegments) results in a correct presentation of all contained media streams. To define such rules the notion of “non-overlapping” segments (respectively Subsegments) is defined as follows.

<!--PB printed 17-->

Let

- T<sub>*E*</sub>(*S*,*i*) be the earliest presentation time of any access unit in stream *i* of a Segment or Subsegment *S*, and
- T<sub>*L*</sub>(*S*,*i*) be the latest presentation time of any access unit in stream *i* of a Segment or Subsegment *S*.

Then two segments (respectively Subsegments), *A* and *B*, which may or may not be of different Representations, are *non-overlapping* if T<sub>*L*</sub>(*A*,*i*) < T<sub>*E*</sub>(*B*,*i*) for all media streams *i* in A and B or if T<sub>*L*</sub>(*B*,*i*) < T<sub>*E*</sub>(*A*,*i*) for all streams *i* in A and B where *i* refers to the same media component.

The property of “non-overlapping” segments (respectively Subsegments) is used to define the terms Segment alignment and Subsegment alignment.

<a id="c4-5-4"></a>

### 4.5.4 Conforming Segment track

A sequence of Segments (respectively Subsegments) is a “conforming Segment (respectively Subsegment) track” if the concatenation of all Segments (respectively Subsegments) in the sequence of Segments (respectively Subsegments) results in a bitstream that conforms to the media formats in use (including container and codecs).

NOTE This implies that a player conforming to the media format can play the resulting bitstream.

<a id="c4-6"></a>

## 4.6 Brands

The ISO base media file format, ISO/IEC 14496-12, defines the concept of brands; brand values identify specifications or conformance points. This document specifies several brands, as listed in Table 1.

Table 1 — Brands defined in this document

| Brand identifier | Clause in this document | Informative description |
|---|---|---|
| `emsg` | 5.10.3.3 | Event message box. |
| `miss` | 6.2.6 | Missing content segment |
| `msdh` | 6.3.4.2 | Media Segment conforming to the general format type for ISO base media file format. |
| `msix` | 6.3.4.3 | Media Segment conforming to the Indexed Media Segment format type for ISO base media file format. |
| `dsms` | 6.3.5.1 | Media Segment conforming to the DASH Self-Initializing Media Segment format type for ISO base media file format. |
| `dash` | 6.3.5.2 | ISO base media file format file specifically designed for DASH including movie fragments and Segment Index. |
| `sims` | 6.4.4.4 | Media Segment conforming to the Sub-Indexed Media Segment format type for ISO base media file format. |
| `sisx` | 6.4.6.2 | Single Index Segment used to index MPEG-2 TS based Media Segments. |
| `risx` | 6.4.6.3 | Representation Index Segment used to index MPEG-2 TS based Media Segments. |
| <!--PB printed 18--> `ssss` | 6.4.6.4 | Subsegment Index Segment used to index MPEG-2 TS based Media Segments. |
| `lmsg` | 7.3.1 | Last Media Segment indicator for ISO base media file format. |
| `dums` | 8.11.3 | Segment sequence indicator in ISO base media file format broadcast TV profile. |

<a id="c4-7"></a>

## 4.7 Schemes

This document specifies several schemes as listed in Table 2.

Table 2 — Schemes defined in this document

| Scheme identifier | Clause in this document | Informative description |
|---|---|---|
| `urn:mpeg:dash:schema:mpd:2011` | Annex B | The namespace of the XML schema for the MPD. |
| `urn:mpeg:dash:period-continuity:2015` | 5.3.2.4 | Period continuity signalling. |
| `urn:mpeg:dash:period-connectivity:2015` | 5.3.2.4 | Period connectivity signalling. |
| `urn:mpeg:dash:adaptation-set-switching:2016` | 5.3.3.5 | Scheme Identifier for switching across Adaptation Sets. |
| `urn:mpeg:dash:preselection:2016` | 5.3.11.2 | Preselection descriptor. |
| `urn:mpeg:dash:reset:2016` | 5.4.2 | MPD reset indicator. |
| `urn:mpeg:dash:resolve-to-zero:2013` | 5.5.3 | xlink resolution to zero element. |
| `urn:mpeg:dash:mp4protection:2011` | 5.8.5.2 | Protection schemes identified by the Scheme Type within the Scheme Type Box of the Protection Scheme Information Box of ISO/IEC 14496-12. |
| `urn:mpeg:dash:13818:1:CA_descriptor:2011` | 5.8.5.2 | Conditional Access System used for ISO/IEC 13818-1 (MPEG-2 Transport Stream). |
| `urn:mpeg:dash:14496:10:frame_packing_arrangement_type:2011` | 5.8.5.3 | Frame-packing arrangement. |
| `urn:mpeg:dash:13818:1:stereo_video_format_type:2011` | 5.8.5.3 | Frame-packing arrangement. |
| `urn:mpeg:dash:23003:3:audio_channel_configuration:2011` | 5.8.5.4 | Channel configuration. Legacy format for backward-compatibility, it is recommended to use the signalling as defined in ISO/IEC 23091-3 instead. |
| `urn:mpeg:dash:outputChannelPositionList:2012` | 5.8.5.4 | A list of output channel position to signal individual speaker positions as defined in ISO/IEC 23001-8. Legacy format for backward-compatibility, it is recommended to use the signalling as defined in ISO/IEC 23091-3 instead. |
| <!--PB printed 19--> `urn:mpeg:dash:role:2011` | 5.8.5.5 | DASH role scheme. |
| `urn:mpeg:dash:stereoid:2011` | 5.8.5.6 | Scheme for multiple views media content description. |
| `urn:mpeg:dash:utc:ntp:2014` | 5.8.5.7 | UTC Timing scheme for NTP servers. |
| `urn:mpeg:dash:utc:sntp:2014` | 5.8.5.7 | UTC Timing scheme for SNTP servers. |
| `urn:mpeg:dash:utc:http-head:2014` | 5.8.5.7 | UTC Timing scheme for HTTP date headers. |
| `urn:mpeg:dash:utc:http-xsdate:2014` | 5.8.5.7 | UTC Timing scheme for HTTP server with xsdate format. |
| `urn:mpeg:dash:utc:http-iso:2014` | 5.8.5.7 | UTC Timing scheme for HTTP server with ISO timing format. |
| `urn:mpeg:dash:utc:http-ntp:2014` | 5.8.5.7 | UTC Timing scheme for HTTP server with NTP format. |
| `urn:mpeg:dash:utc:direct:2014` | 5.8.5.7 | UTC Timing scheme for direct inclusion of time. |
| `urn:mpeg:dash:audio-receiver-mix:2014` | 5.8.5.8 | Scheme identifier for receiver mix. |
| `urn:mpeg:dash:mpd-as-linking:2015` | 5.8.5.9 | MPD Adaptation Set Linking scheme. |
| `urn:mpeg:dash:sai:2015` | 5.8.5.10 | Sub-Asset Scheme Identifier. |
| `urn:mpeg:dash:client-authentication:2015` | 5.8.5.11 | Client Authentication scheme. |
| `urn:mpeg:dash:content-authorization:2015` | 5.8.5.11 | Content Access Authorization scheme. |
| `urn:mpeg:dash:audio-interactivity:2016` | 5.8.5.12 | Scheme to indicate content interactivity. |
| `urn:mpeg:dash:qr-equivalence:2019` | 5.8.5.13 | Quality Equivalence Descriptor |
| `urn:mpeg:dash:output-protection:hdcp:2020` | 5.8.5.14.2 | Scheme to indicate the HDCP output protection |
| `urn:mpeg:dash:event:2012` | 5.10.4 | DASH event signalling scheme. |
| `urn:mpeg:dash:event:callback:2015` | 5.10.4.5 | DASH call back event. |
| `urn:mpeg:dash:event:ttfn:2016` | 5.10.4.6 | Presentation Termination Event. |
| `urn:mpeg:dash:mpd-chaining:2016` | 5.11.1 | MPD chaining descriptor. |
| `urn:mpeg:dash:fallback:2016` | 5.11.2 | MPD fallback descriptor. |
| `urn:mpeg:dash:profile:full:2011` | 8.2 | Identifier for Full profile. |
| `urn:mpeg:dash:profile:isoff-on-demand:2011` | 8.3 | Identifier for ISO Base media file format On Demand profile. |
| <!--PB printed 20--> `urn:mpeg:dash:profile:isoff-live:2011` | 8.4 | Identifier for ISO Base media file format live profile. |
| `urn:mpeg:dash:profile:isoff-main:2011` | 8.5 | Identifier for ISO Base media file format main profile. |
| `urn:mpeg:dash:profile:mp2t-main:2011` | 8.6 | Identifier for MPEG-2 TS main profile. |
| `urn:mpeg:dash:profile:mp2t-simple:2011` | 8.7 | Identifier for MPEG-2 TS simple profile. |
| `urn:mpeg:dash:profile:isoff-ext-live:2014` | 8.8 | Identifier for ISO Base media file format extended live profile. |
| `urn:mpeg:dash:profile:isoff-ext-on-demand:2014` | 8.9 | Identifier for ISO Base media file format extended On Demand profile. |
| `urn:mpeg:dash:profile:isoff-common:2014` | 8.10 | Identifier for ISO Base media file format common profile. |
| `urn:mpeg:dash:profile:isoff-broadcast:2015` | 8.11 | Identifier for ISO Base media file format broadcast TV profile |
| `urn:mpeg:dash:profile:cmaf:2019` | 8.12 | Identifier for DASH core profile for CMAF content |
| `urn:mpeg:dash:profile:cmaf-extended:2019` | 8.12 | Identifier for DASH extended profile for CMAF content |
| `urn:mpeg:dash:event:catchall:2020` | A.13 | URI for subscribing to all application event and timed metadata received by DASH Client |
| `urn:mpeg:dash:srd:2014` | H.1 | Scheme identifier for Spatial Relationship Description. If signalled in an Adaptation Set, the Adaptation Set shall follow the description in Annex H. |
| `urn:mpeg:dash:srd:dynamic:2016` | H.1 | Scheme to signal dynamic SRD. If signalled in an Adaptation Set, the Adaptation Set shall follow the description in Annex H. |
| `urn:mpeg:dash:urlparam:2014` | I.2 | Scheme identifier for indicating usage of the flexible insertion of URL query parameters. If signalled in an Adaptation Set, the Adaptation Set shall follow the description in Annex I. |
| `urn:mpeg:dash:urlparam:2016` | I.3 | Scheme identifier for indicating usage of the extended parameterization scheme. If signalled in an Adaptation Set, the Adaptation Set shall follow the description in Annex I. |
| `urn:mpeg:dash:resolutionSwitching:2016` | Annex J | Descriptor for Open GOP resolution change |

<!--PB printed 21-->

## Transcription notes

> **Spec note:** 4.3: "self-initialitizing" is a typo in the PDF, reproduced as printed.
>
> **Spec note:** 4.5.2: the NOTE and its six "Type n" bullets are one NOTE in the PDF; "ISAP", "TSAP", ... are subscripted symbols (I_SAP, T_SAP, ...), written here with `<sub>`.
>
> **Spec note:** Table 1 (brands) and Table 2 (schemes) were rebuilt from the rendered pages; identifiers that wrap across lines in the PDF cells are joined.
