---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clauses 5.3.9 to 5.3.9.3 - Segments and Segment information: general, Segment base information, Segment list"
pdf_pages: "79-90"
printed_pages: "71-82"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-9"></a>

### 5.3.9 Segments and Segment information

<a id="c5-3-9-1"></a>

#### 5.3.9.1 General

This clause defines the MPD information for Segments. Segment formats are defined in 6.

Specifically, a Segment shall either be

- referenced by an HTTP-URL included in the MPD, where an HTTP-URL is defined as an <absolute-URI> according to IETF RFC 3986:2005 subclause [4.3](04-overview.md#c4-3), with a fixed scheme of “http” or “https”, possibly restricted by a byte range if a range attribute is provided together with the URL. The byte range shall be expressed as a byte-range-spec as defined in IETF RFC 2616 subclause 14.35.1. It is restricted to a single expression identifying a contiguous range of bytes, or

<!--PB printed 71-->

- included in the MPD with a data URL as defined in IETF RFC 2397. Data URLs shall only be used if explicitly permitted by the profile. In addition, the guidelines and recommendations in IETF RFC 2397 should be taken into account.

Each Segment referenced through an HTTP-URL in the MPD is associated with a Segment availability interval, i.e. a time window in wall-clock time at which the Segment can be accessed via the HTTP-URL. The Segment availability interval window is described by a Segment availability start time and a Segment availability end time.

Representations are assigned Segment Information through the presence of the elements BaseURL,

SegmentBase, SegmentTemplate and/or SegmentList. The Segment Information provides information on the location, availability and properties of all Segments contained in one Representation. Specifically, information on the presence and location of Initialization, Media, Index and Bitstream Switching Segments is provided.

The elements SegmentBase, SegmentTemplate and SegmentList may be present in the Representation element itself. In addition, to express default values, they may be present in the Period and AdaptationSet element. At each level at most one of the three, SegmentBase,

SegmentTemplate and SegmentList, shall be present. Further, if SegmentTemplate or SegmentList is present on one level of the hierarchy, then the other one shall not be present on any lower hierarchy level.

SegmentBase, SegmentTemplate and SegmentList shall inherit attributes and elements from the same element on a higher level. If the same attribute or element is present on both levels, the one on the lower level shall take precedence over the one on the higher level.

Several mechanisms are available to specify the Segment Information. Specifically, each Representation shall have assigned exactly one of the following choices to determine the Segment Information, either by direct presence in the Representation element or by inheritance from the higher levels:

- one SegmentList element — for syntax and semantics, refer to subclause [5.3.9.3](#c5-3-9-3),
- one SegmentTemplate element — for syntax and semantics, refer to subclause [5.3.9.4](05-3-9-4-segment-template.md#c5-3-9-4),
- one or more BaseURL elements, at most one SegmentBase element, and no SegmentTemplate or SegmentList element. The SegmentBase element is defined in subclause [5.3.9.2.](#c5-3-9-2)

NOTE These rules do not prohibit the usage of the BaseURL element together with SegmentList or SegmentTemplate. If the BaseURL is present together with the either the SegmentList or the SegmentTemplate, then processing according to subclause 5.6 applies. All three elements SegmentBase, SegmentTemplate and SegmentList share common elements based on the SegmentBase element. Furthermore, SegmentTemplate and SegmentList share common attributes and elements. The common information is defined in subclause [5.3.9.2.](#c5-3-9-2) The derivation and details of Initialization, Media, Index and Bitstream Switching Segment Information based on the above information are provided in subclause [5.3.9.5.](05-3-9-5-segment-information-derivation.md#c5-3-9-5)

<a id="c5-3-9-2"></a>

#### 5.3.9.2 Segment base information

<a id="c5-3-9-2-1"></a>

##### 5.3.9.2.1 Overview

The SegmentBase element is sufficient to describe the Segment Information if and only if a single Media Segment is provided per Representation and the Media Segment URL is included in the BaseURL element.

<!--PB printed 72-->

In case multiple Media Segments are present, either a SegmentList or a SegmentTemplate shall be used to describe the Segment Information. SegmentList or a SegmentTemplate share the multiple Segment base information as provided in subclause [5.3.9.2.2](#c5-3-9-2-2), Table 17.

If the Representation contains more than one Media Segment, then either the attribute @duration or the element SegmentTimeline shall be present. The attribute @duration and the element SegmentTimeline shall not be present at the same time.

Segments described by the Segment base information are referenced by an HTTP-URL conforming to the type URLType as defined in Table 18.

The semantics of the attributes and elements for the SegmentBase element and the Segment base information are provided in subclause [5.3.9.2.2](#c5-3-9-2-2), Table 16 and the multiple Segment base information in Table 17 in subclause [5.3.9.2.2.](#c5-3-9-2-2) The XML syntax of the Segment Base Information is provided in subclause [5.3.9.2.3.](#c5-3-9-2-3)

<a id="c5-3-9-2-2"></a>

##### 5.3.9.2.2 Semantics

Table 16 — Semantics of SegmentBase element and Segment Base Information type

| Element or Attribute Name | Use | Description |
|---|---|---|
| **SegmentBase**<br>***Segment Base Information*** |  | specifies Segment base element.<br><br>This element also specifies the type for the Segment base information that is the base type for other elements. |
| `@timescale` | O | specifies the timescale in units per seconds to be used for the derivation of different real-time duration values in the Segment Information.<br><br>If not present on any level, it shall be set to 1.<br><br>NOTE This can be any frequency but typically is the media clock frequency of one of the media streams (or a positive integer multiple thereof). |
| `@presentationTimeOffset` | O | specifies the presentation time offset of the Representation relative to the start of the Period, i.e. the presentation time value of the media stream that shall be presented at the start of this Period.<br><br>The value of the presentation time offset in seconds is the division of the value of this attribute and the value of the `@timescale` attribute.<br><br>If not present on any level, the value of the presentation time offset is 0. |
| <!--PB printed 73--> `@eptDelta` | O | specifies the difference between the earliest presentation time in the Representation and the value of the `@presentationTimeOffset`.<br><br>The value of the earliest presentation time of the first Media Segment in this Representation in seconds is computed as the sum of the value of this attribute and the value of the `@presentationTimeOffset` in units of the `@timescale` attribute.<br><br>If the media is contained in a Self-Initializing Media Segment, the value of `@presentationTimeOffset` is used to identify the Sub-Segment, that is the first one of the Representation. In this case, the value of `@eptDelta` shall be set to the difference of the `@presentationTimeOffset` and the earliest presentation time of the Subsegment containing the media time provided by the value of `@presentationTimeOffset`.<br><br>If not present, the value is unknown, but specific profiles may require it to be zero, if not signalled.<br><br>NOTE If the value if `@eptDelta` is smaller than zero then this results in an overlap prior to the Period. If the value is positive, it results in a gap. |
| `@pdDelta` | O | specifies the difference between the presentation duration of this Representation in units of `@timescale` and the Period duration.<br><br>If the media is contained in a Self-Initializing Media Segment, the sum of the value of `@presentationTimeOffset` and the Period duration is used to identify the Subsegment, that is the last one of the Representation. In this case, `@pdDelta` shall be set according to the presentation duration of this last Subsegment.<br><br>If not present, the value is unknown.<br><br>NOTE If the value if `@pdDelta` is smaller than zero then this results in a gap at the end of the Period. If the value is positive, it results in an overlap. |
| <!--PB printed 74--> `@presentationDuration` | O | specifies the presentation duration of the Representation in the Period.<br><br>The value of the presentation duration in seconds is the division of the value of this attribute and the value of the `@timescale` attribute. Specifically, the sum of the value of the `@presentationTimeOffset`, if present, or 0 otherwise and the value of this attribute is the last presentation time to be presented for this Representation.<br><br>If not present on any level, the value of this attribute is unknown and the Representation should be presented until the end of the Period, i.e. until the presentation is terminated or until the next Period starts. |
| `@timeShiftBufferDepth` | O | specifies the duration of the time shifting buffer for this Representation that is guaranteed to be available for a Media Presentation with type 'dynamic'. When not present, the value is of the `@timeShiftBufferDepth` on MPD level applies. If present, this value shall be not smaller than the value on MPD level. This value of the attribute is undefined if the `@type` attribute is equal to 'static'.<br><br>NOTE When operating in a time-shift buffer on a Representation with value larger than the time-shift buffer signalled on MPD level, not all Representations are necessarily available for switching. |
| `@indexRange` | O | specifies the byte range that contains the Segment Index in all Media Segments of the Representation.<br><br>The byte range shall be expressed and formatted as a `byte-range-spec` as defined in IETF RFC 7233:2014, subclause 2.1. It is restricted to a single expression identifying a contiguous range of bytes.<br><br>If not present, the value is unknown. |
| `@indexRangeExact` | OD<br>default<br>"false" | when set to `'true'` specifies that for all Segments in the Representation, the data outside the prefix defined by `@indexRange` contains the data needed to access all access units of all media streams syntactically and semantically.<br><br>This attribute shall not be present if `@indexRange` is absent. |
| <!--PB printed 75--> `@availabilityTimeOffset` | O | specifies an offset to define the adjusted segment availability time. The value is specified in seconds, possibly with arbitrary precision.<br><br>The offset provides the time how much earlier these segments are available compared to their computed availability start time for all Segments of all associated Representation.<br><br>The segment availability start time defined by this value is referred to as adjusted segment availability start time. For details on computing the adjusted segment availability start time, refer to subclause [5.3.9.5.](05-3-9-5-segment-information-derivation.md#c5-3-9-5)<br><br>If not present, no adjusted segment availability start time is defined.<br><br>NOTE The value of "INF" implies availability of all segments starts at **MPD**`@availabilityStartTime`. |
| `@availabilityTimeComplete` | O | specifies if all Segments of all associated Representation are complete at the adjusted availability start time. The attribute shall be ignored if `@availabilityTimeOffset` is not present on any level.<br><br>If not present on any level, the value is inferred to true.<br><br>NOTE If the value is set to false, then it can be inferred by the client that the Segment is available at its announced location prior being complete. |
| **Initialization** | 0 ... 1 | specifies the URL including a possible byte range for the Initialization Segment.<br><br>For the type definition, refer to Table 18. |
| **RepresentationIndex** | 0 ... 1 | specifies the URL including a possible byte range for the Representation Index Segment.<br><br>For the type definition, refer to Table 18. |
| **FailoverContent** | 0 ... 1 | specifies times where the content has been replaced by failover content, for example because of an encoder error.<br><br>For details refer to subclause [5.3.9.7.](05-3-9-7-failover-content.md#c5-3-9-7) |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold and preceded with an @.

<!--PB printed 76-->

Table 17 — Semantics of MultipleSegmentBaseInformation type

| Element or Attribute Name | Use | Description |
|---|---|---|
| ***MultipleSegmentBaseInformation*** |  | specifies multiple Segment base information. |
| `@duration` | O | If present, specifies the constant approximate Segment duration.<br><br>All Segments within this Representation element have the same duration unless it is the last Segment within the Period, which can be significantly shorter.<br><br>The value of the duration in seconds is the division of the value of this attribute and the value of the `@timescale` attribute associated to the containing Representation.<br><br>For more details, refer to subclause [5.3.9.5.3.](05-3-9-5-segment-information-derivation.md#c5-3-9-5-3) |
| `@startNumber` | O | specifies the number of the first Media Segment in this Representation in the Period.<br><br>For details, refer to subclause [5.3.9.5.3.](05-3-9-5-segment-information-derivation.md#c5-3-9-5-3) |
| `@endNumber` | O | specifies the number of the last Media Segment in this Representation in the Period.<br><br>If not present, the number is inferred from the duration of the Period.<br><br>For details refer to 5.3.9.5.3. |
| ***Segment Base Information*** |  | specifies Segment base information. |
| **SegmentTimeline** | 0...1 | specifies the timeline of arbitrary Segment durations<br><br>For more details, see subclause [5.3.9.6.](05-3-9-6-segment-timeline.md#c5-3-9-6) |
| **BitstreamSwitching** | 0 ... 1 | specifies the URL including a possible byte range for the Bitstream Switching Segment.<br><br>For the type definition, refer to Table 18. |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold and preceded with an @.

<!--PB printed 77-->

Table 18 — Semantics of elements of type URLType

| Element or Attribute Name | Use | Description |
|---|---|---|
| **Element of type URLType** |  | defines an HTTP-URL |
| `@sourceURL` | O | specifies the source URL part and shall be formatted either as an <absolute-URI> according to IETF RFC 3986:2005, subclause [4.3](04-overview.md#c4-3), with a fixed scheme of “http” or “https” or as a <relative-ref> according to IETF RFC 3986:2005 subclause [4.2.](04-overview.md#c4-2)<br><br>If not present, then any BaseURL element is mapped to the `@sourceURL` attribute and the range attribute shall be present. |
| `@range` | O | specifies the byte range restricting the above HTTP-URL.<br><br>The byte range shall be expressed and formatted as a `byte-range-spec` as defined in IETF RFC 7233:2014 subclause 2.1. It is restricted to a single expression identifying a contiguous range of bytes.<br><br>If not present, the element refers to the entire resource referenced in the `@sourceURL` attribute. |

<a id="c5-3-9-2-3"></a>

##### 5.3.9.2.3 XML-Syntax

```xml
<xs:complexType name="SegmentBaseType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment information base</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="Initialization" type="URLType" minOccurs="0"/>
        <xs:element name="RepresentationIndex" type="URLType" minOccurs="0"/>
        <xs:element name="FailoverContent" type="FailoverContentType" minOccurs="0"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="timescale" type="xs:unsignedInt"/>
    <xs:attribute name="eptDelta" type="xs:integer"/>
    <xs:attribute name="pdDelta" type="xs:integer"/>
    <xs:attribute name="presentationTimeOffset" type="xs:unsignedLong"/>
    <xs:attribute name="presentationDuration" type="xs:unsignedLong"/>
    <xs:attribute name="timeShiftBufferDepth" type="xs:duration"/>
    <xs:attribute name="indexRange" type="SingleRFC7233RangeType"/>
    <xs:attribute name="indexRangeExact" type="xs:boolean" default="false"/>
    <xs:attribute name="availabilityTimeOffset" type="xs:double"/>
    <xs:attribute name="availabilityTimeComplete" type="xs:boolean"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:complexType name="MultipleSegmentBaseType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Multiple Segment information base</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="SegmentBaseType">
            <xs:sequence>
                <xs:element name="SegmentTimeline" type="SegmentTimelineType" minOccurs="0"/>
                <xs:element name="BitstreamSwitching" type="URLType" minOccurs="0"/>
            </xs:sequence>
            <xs:attribute name="duration" type="xs:unsignedInt"/>
            <xs:attribute name="startNumber" type="xs:unsignedInt"/>
            <xs:attribute name="endNumber" type="xs:unsignedInt"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
<xs:complexType name="URLType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment Info item URL/range</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="sourceURL" type="xs:anyURI"/>
    <xs:attribute name="range" type="SingleRFC7233RangeType"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="SingleRFC7233RangeType">
    <xs:restriction base="xs:string">
        <xs:pattern value="([0-9]*)(\-([0-9]*))?"/>
    </xs:restriction>
</xs:simpleType>
```
<!--PB printed 78-->
<!--(page breaks fell inside the XML block above)-->

<a id="c5-3-9-3"></a>

#### 5.3.9.3 Segment list

<a id="c5-3-9-3-1"></a>

##### 5.3.9.3.1 Overview

The Segment list is defined by a SegmentList element. Each SegmentList element itself contains a list of SegmentURL elements for a consecutive list of Segment URLs. Each Segment URL may contain the Media Segment URL and possibly a byte range. The Segment URL element may also contain an Index Segment.

The semantics of the attributes and elements for the Segment list are provided in subclause [5.3.9.3.2](#c5-3-9-3-2), Table 19. The XML syntax of the Segment Information is provided in subclause [5.3.9.3.3.](#c5-3-9-3-3)

<a id="c5-3-9-3-2"></a>

##### 5.3.9.3.2 Semantics

Table 19 — Semantics of SegmentList element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **SegmentList** |  | specifies Segment information. |
| `@xlink:href` | O | specifies a reference to a remote element entity that contains one or multiple elements of type **SegmentList**. |
| `@xlink:actuate` | OD<br>default: "onRequest" | specifies the processing set, can be either "onLoad" or "onRequest" |
| ***MultipleSegmentBaseInformation*** |  | Multiple Segment base information as defined in subclause [5.3.9.2](#c5-3-9-2), Table 17. |
| **SegmentURL** | 0 ... N | specifies a Media Segment URL and a possibly present Index Segment URL |
| <!--PB printed 79--> `@media` | O | in combination with the `@mediaRange` attribute, specifies the HTTP-URL for the Media Segment.<br><br>It shall be formatted as an <absolute-URI> according to IETF RFC 3986:2005, subclause [4.3](04-overview.md#c4-3), with a fixed scheme of “http” or “https” or as a <relative-ref> according to IETF RFC 3986:2005, subclause [4.2.](04-overview.md#c4-2)<br><br>If not present, then any **BaseURL** element is mapped to the @media attribute and the range attribute shall be present. |
| `@mediaRange` | O | specifies the byte range within the resource identified by the `@media` corresponding to the Media Segment.<br><br>The byte range shall be expressed and formatted as a `byte-range-spec` as defined in IETF RFC 7233:2014, subclause 2.1. It is restricted to a single expression identifying a contiguous range of bytes.<br><br>If not present, the Media Segment is the entire resource referenced by the `@media` attribute. |
| `@index` | O | in combination with the `@indexRange` attribute, specifies the HTTP-URL for the Index Segment.<br><br>It shall be formatted as an <absolute-URI> according to IETF RFC 3986:2005, subclause [4.3](04-overview.md#c4-3), with a fixed scheme of “http” or “https” or as a <relative-ref> according to IETF RFC 3986:2005, subclause [4.2.](04-overview.md#c4-2)<br><br>If not present and the `@indexRange` not present either, then no Index Segment information is provided for this Media Segment.<br><br>If not present and the `@indexRange` present, then the `@media` attribute is mapped to the `@index`. If the `@media` attribute is not present either, then any **BaseURL** element is mapped to the `@index` attribute and the `@indexRange` attribute shall be present. |
| <!--PB printed 80--> `@indexRange` | O | specifies the byte range within the resource identified by the `@index` corresponding to the Index Segment. If `@index` is not present, it specifies the byte range of the Segment Index in Media Segment.<br><br>The byte range shall be expressed and formatted as a `byte-range-spec` as defined in IETF RFC 7233:2014, subclause 2.1. It is restricted to a single expression identifying a contiguous range of bytes.<br><br>If not present, the Index Segment is the entire resource referenced by the `@index` attribute. |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

The conditions only hold without using `@xlink:href`. If linking is used, then all attributes are "optional" and <minOccurs=0>.

Elements are **bold**; attributes are non-bold and preceded with an @.

<a id="c5-3-9-3-3"></a>

##### 5.3.9.3.3 XML-Syntax

```xml
<xs:complexType name="SegmentListType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment List</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="MultipleSegmentBaseType">
            <xs:sequence>
                <xs:element name="SegmentURL" type="SegmentURLType" minOccurs="0" maxOccurs="unbounded"/>
            </xs:sequence>
            <xs:attribute ref="xlink:href"/>
            <xs:attribute ref="xlink:actuate" default="onRequest"/>
            <xs:attribute ref="xlink:type" fixed="simple"/>
            <xs:attribute ref="xlink:show" fixed="embed"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
<xs:complexType name="SegmentURLType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Segment URL</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="media" type="xs:anyURI"/>
    <xs:attribute name="mediaRange" type="SingleRFC7233RangeType"/>
    <xs:attribute name="index" type="xs:anyURI"/>
    <xs:attribute name="indexRange" type="SingleRFC7233RangeType"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```

<!--PB printed 81-->
