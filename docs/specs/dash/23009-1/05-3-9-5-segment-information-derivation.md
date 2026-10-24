---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.9.5 - Segment information (Initialization, Media, Index, Bitstream Switching)"
pdf_pages: "91-95"
printed_pages: "83-87"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-9-5"></a>

#### 5.3.9.5 Segment information

<a id="c5-3-9-5-1"></a>

##### 5.3.9.5.1 Overview

The Segment Information provides the following information:

- the presence or absence of Initialization, Index and Bitstream Switching Segment information,
- the HTTP-URL and possibly a byte range for each accessible Segment in each Representation, or the Segment directly in the MPD as a data URL as defined in IETF RFC 2397,
- all valid Segment URLs declared by the containing MPD,
- for services with MPD@type='dynamic', the Segment availability start time and Segment availability end time of each Segment,
- an approximate Media Presentation start time of each Media Segment in the Media Presentation timeline within the Period.

The derivation of Initialization, Media, Index and Bitstream Switching Segment Information from the elements SegmentBase, SegmentList and SegmentTemplate is provided in subclauses [5.3.9.5.2](#c5-3-9-5-2), [5.3.9.5.3](#c5-3-9-5-3), [5.3.9.5.4](#c5-3-9-5-4) and [5.3.9.5.5.](#c5-3-9-5-5) Reference resolution as defined in subclause 5.6.4 and base URL selection as defined in subclause 5.6.5 using BaseURL elements as defined in subclause 5.6 shall be applied to any URLs.

<a id="c5-3-9-5-2"></a>

##### 5.3.9.5.2 Initialization Segment information

Each Representation has assigned at most one Initialization Segment.

The presence of an Initialization Segment is indicated by the presence of SegmentBase.Initialization, SegmentList.Initialization, the SegmentTemplate.Initialization element or the SegmentTemplate@initialization attribute that may contain URL and byte range information or URL construction rules for the Initialization Segment.

If neither Initialization element nor SegmentTemplate@initialization attribute are present for a Representation then each Media Segment within the Representation shall be self-initializing.

For services with MPD@type='dynamic', the Segment availability start time of the Initialization Segment is the sum of the value of the MPD@availabilityStartTime and the PeriodStart time as defined in subclause [5.3.2.1](05-3-2-period.md#c5-3-2-1) of the containing Period and the Segment availability end time of the

<!--PB printed 84-->

Initialization Segment is the largest Segment availability end time of any Media Segment in this Representation. For Segment availability of Media Segments, refer to subclause [5.3.9.5.3.](#c5-3-9-5-3)

The data structures retrieved from the Initialization URL are defined in subclause 6.2.2.

<a id="c5-3-9-5-3"></a>

##### 5.3.9.5.3 Media Segment information

If a Representation consists of more than one Media Segment, then this Representation has assigned a list of consecutive Media Segments. The list may be specified explicitly by one or more SegmentList elements or implicitly by a SegmentTemplate element.

Each entry in this Media Segment list has assigned the following parameters:

- a valid Media Segment URL and possibly a byte range,
- the number and position of the Media Segment in the Representation,
- the MPD start time of the Media Segment in the Representation providing an approximate presentation start time of the Segment in the Period,
- the MPD duration of the Media Segment providing an approximate presentation duration of the Segment.

The MPD start time and the MPD duration may be approximate and do not necessarily reflect the exact Media Presentation time. For more details on the relation of MPD start times and Media Presentation time, refer to subclause 7.2.1.

In order to obtain the list of Media Segment URLs, i.e. the URL for each Segment at a specific position k in the list based on the Segment Information, the following shall apply:

- If SegmentTemplate element is present, the Template-based Segment URL construction in subclause [5.3.9.4.4](05-3-9-4-segment-template.md#c5-3-9-4-4) shall be applied as follows.
  - If the Representation contains or inherits a SegmentTemplate element with $Number$ then the URL of the Media Segment at position k in the Representation is determined by replacing the $*Number*$ identifier by (*k*−1) + (*k*<sub>start</sub>−1) with *k*<sub>start</sub> the value of the @startNumber attribute, if present, or 1 otherwise.
  - If the Representation contains or inherits a SegmentTemplate element with $Time$ then the URL of the media segment at position k is determined by replacing the $Time$ identifier by MPD start time of this segment, as described below.
- If one or more SegmentList elements are present then they contain a list of SegmentURL elements for a consecutive list of Media Segment URLs. The number of the first Segment in the list within this Period is determined by the value of the SegmentList@startNumber attribute, if present, or it is 1 in case this attribute is not present. The sequence of multiple SegmentList elements within a Representation shall result in Media Segment List with consecutive numbers.
- None of the above: in this case, only a single Media Segment shall be present with the URL provided by a BaseURL element and the SegmentBase element may be present.

For the derivation of the MPD start time and duration of each Media Segment in the list of Media Segments, the position k of the Media Segment and the following information are used.

- If neither @duration attribute nor SegmentTimeline element is present, then the Representation shall contain exactly one Media Segment. The MPD start time is 0 and the MPD duration is obtained in the same way as for the last Media Segment in the Representation (see below for more details).

<!--PB printed 85-->

- If @duration attribute is present, then the MPD start time of the Media Segment is determined as (*Number*-*Number*<sub>Start</sub>) times the value of the attribute @duration with *Number*<sub>Start</sub> the value of the @startNumber attribute, and *Number* the segment number [e.g. (*k*−1) + *Number*<sub>Start</sub>]. The MPD duration of the Media Segment is the value of the attribute @duration unless the Media Segment is the last one the Representation (see below for more details).
- If @duration attribute is not present and the SegmentTimeline element is present then rules in subclause [5.3.9.6](05-3-9-6-segment-timeline.md#c5-3-9-6) apply to determine the start time and duration of each Media Segment in the Media Segment list.
- To determine the duration of the only or the last Media Segment of any Representation in a Period, the MPD shall include sufficient information to determine the duration of the containing Period. For example, the MPD@mediaPresentationDuration, or Period@duration, or next Period@start may be present. The Media Presentation author should always provide a Segment as indicated by the duration of the Period, even if the Segment may not contain any media samples. If the content author provides a different number of Segments as nominally derived, it should add the @endNumber attribute to provide the number of the last segment in the Period. The value of the @endNumber should differ by at most 1 from the nominally derived value. A Segment with a number greater than the value of @endNumber is not valid.

For services with MPD@type='dynamic', the Segment availability start time of a Media Segment is the sum of

- the value of the MPD@availabilityStartTime,
- the PeriodStart time of the containing Period as defined in subclause [5.3.2.1](05-3-2-period.md#c5-3-2-1),
- the MPD start time of the Media Segment, and
- the MPD duration of the Media Segment.

NOTE By adding the MPD duration of the segment to the segment availability start time of the segment, the segment availability start time of the first segment of each Period depends on the segment duration. This enables to provide segments in Representations with shorter MPD duration earlier, for example to reduce latency for certain Representations.

The Segment availability end time of a Media Segment is the sum of the Segment availability start time, the MPD duration of the Media Segment and the value of the attribute @timeShiftBufferDepth for this Representation.

If the @availabilityTimeOffset attribute is present for a Representation in the Segment Information or the BaseURL element, then the parameter availabilityTimeOffset is determined as the sum of all values of @availabilityTimeOffset on all levels that are processed in determining the URL for the corresponding segment. Then the adjusted segment availability start time is determined by subtracting the value of availabilityTimeOffset from the Segment availability start time. This adjusted segment availability start time provides a time instant in wall-clock time at which a Segment becomes an available Segment. If the @availabilityTimeComplete flag is set to false for such a Representation on any level, then the entire Segment may not yet be fully available at the adjusted segment availability start time. No promise how the Segment will be made available is provided in this case. To signal the gradual availability of Segments over time, refer to subclause 5.3.12.1.

<!--PB printed 86-->

The MPD shall include URL information for all Segments with an availability start time less than both (i) the end of the Media Presentation and (ii) the sum of the latest time at which this version of the MPD is available on the server and the value of the MPD@minimumUpdatePeriod.

The data structures retrieved from the URL referring to a Media Segment are defined in subclause 6.2.3.

<a id="c5-3-9-5-4"></a>

##### 5.3.9.5.4 Index Segment information

Each Segment typically has assigned Segment Index information that may be provided in an explicitly declared Index Segment.

The presence of explicit Index Segment information is indicated

- by the presence of one RepresentationIndex element providing the Segment Index for the entire Representation, or
- by the presence of at least one of the two attributes @index and @indexRange in the SegmentList.SegmentURL element, or
- by the presence of SegmentTemplate@index attribute. If either $Number$ or $Time$ are present the Template-based Segment URL construction in subclause [5.3.9.4.4](05-3-9-4-segment-template.md#c5-3-9-4-4) shall be applied with number set to the number of the corresponding Media Segment. If not present, the SegmentTemplate@index attribute constitutes a reference to Representation Index.

The @indexRange attribute may also be used to provide the byte range for an index within a Media Segment, where this is allowed by the Media Segment format. In this case the @index attribute shall not be present and the range specified shall lie completely within any byte range specified for the Media Segment.

The availability of Index Segments is identical to the availability to the Media Segments they correspond to.

The data structures retrieved from the URL referring to an Index Segment are defined in subclause 6.2.4.

<a id="c5-3-9-5-5"></a>

##### 5.3.9.5.5 Bitstream Switching Segment information

Each Representation has assigned at most one Bitstream Switching Segment. The Bitstream Switching Segment is only relevant in case the @bitstreamSwitching flag is set to 'true' and may enable the creation of a conforming Segment track for Segments from different Representations.

The presence of a Bitstream Switching Segment is indicated by the presence of the BitstreamSwitching element or the SegmentTemplate@bitstreamSwitching attribute that that may contain URL and byte range information or construction rules for the URL.

If neither BitstreamSwitching element nor SegmentTemplate@bitstreamSwitching attribute are present for a Representation and the @bitstreamSwitching flag is set to 'true', there are no Bitstream Switching Segments.

The Segment availability time of the Bitstream Switching Segment is identical to the one specified for the Initialization Segment in subclause [5.3.9.5.2.](#c5-3-9-5-2)

The data structures retrieved from the URL referring to a Bitstream Switching Segment are defined in subclause 6.2.5.

<!--PB printed 87-->
