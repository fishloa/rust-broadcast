---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.5 - Representation"
pdf_pages: "59-70"
printed_pages: "51-62"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-5"></a>

### 5.3.5 Representation

<a id="c5-3-5-1"></a>

#### 5.3.5.1 Overview

Representations are described by the Representation element. Representation elements are contained in an AdaptationSet element.

A Representation is one of the alternative choices of the complete set or subset of media content components comprising the media content during the defined Period.

A Representation starts at the start of the Period PeriodStart and continues to the end of the Period, i.e. the start of the next Period or the end of the Media Presentation.

Each Representation includes one or more media streams, where each media stream is an encoded version of one media content component.

A Representation consists of one or more Segments.

Each Representation either shall contain an Initialization Segment or each Media Segment in the Representation shall be self-initializing, i.e. the Media Segment itself conforms to the media type as specified in the @mimeType attribute for this Representation.

<!--PB printed 51-->

When a Representation is not a dependent Representation, i.e. the @dependencyId attribute is absent, then concatenation of the Initialization Segment, if present, and all consecutive Media Segments in one Representation shall represent a conforming Segment track as defined in subclause [4.5.4](04-overview.md#c4-5-4) conforming to the media type as specified in the @mimeType attribute for this Representation.

Dependent Representations are described by a Representation element that contains a @dependencyId attribute. Dependent Representations are regular Representations except that they depend on a set of complementary Representations for decoding and/or presentation. The @dependencyId contains the values of the @id attribute of all the complementary Representations, i.e. Representations that are necessary to present and/or decode the media content components contained in this dependent Representation.

For any dependent Representation X that depends on complementary Representation Y, the m-th Subsegment of X and the n-th Subsegment of Y shall be non-overlapping (as defined in subclause [4.5.3](04-overview.md#c4-5-3)) whenever m is not equal to n. For dependent Representations, the concatenation of the Initialization Segment with the sequence of Subsegments of the dependent Representations, each being preceded by the corresponding Subsegment of each of the complementary Representations in order as provided in the @dependencyId attribute, shall represent a conforming Subsegment track as defined in subclause [4.5.4](04-overview.md#c4-5-4) conforming to the media format as specified in the @mimeType attribute for this dependent Representation.

NOTE 1 When decoding of a dependent Representation is started from a SAP in the (Sub)Segment with number i, the decoding process does not need to access data from the complementary Representation(s) from any earlier (sub)segments than (sub)Segment with number i of the complementary Representation(s).

NOTE 2 In case a dependent Representation X depends on at least two complementary Representations Yi, that are also dependent Representations depending, directly or indirectly, on the same complementary Representation Z, then the concatenation as defined above results in a sequence conforming to the media format if the Segments of Representation Z are concatenated only the first time encountered following the order in the @dependencyId attributes starting from X. If following recursively the dependencies indicated in the @dependencyId of Representations, certain Representations can be encountered more than once. In this case, the corresponding Segments or Subsegments are expected to only be concatenated once, namely when first encountered.

Associated Representations are described by a Representation element that contains an @associationId attribute and optionally an @associationType attribute. Associated Representations are Representations that provide information on their relationships with other Representations. As opposed to complementary Representations, the segments of an associated Representation may be optional for decoding and/or presentation of the Representations identified by @associationId. They can be considered as supplementary or descriptive information, the type of the association being specified by the @associationType attribute.

NOTE 3 @associationId and @associationType attributes can only be used between Representations that are not in the same Adaptation Sets.

If a Representation is offered in a Media Presentation with MPD@type='dynamic', it is recommended that means to compensate such drift be included. For more details, refer to A.8.

The semantics of the attributes and elements within a Representation are provided in Table 9 in subclause [5.3.5.2](#c5-3-5-2). The XML syntax of the Representation type is provided in subclause [5.3.5.3](#c5-3-5-3).

<!--PB printed 52-->

<a id="c5-3-5-2"></a>

#### 5.3.5.2 Semantics

Table 9 — Semantics of Representation element

| Element or Attribute Name | Use | Description |
|---|---|---|
| **Representation** |  | This element contains a description of a Representation. |
| `@id` | M | specifies an identifier for this Representation. The identifier shall be unique within a Period unless the Representation is functionally identical to another Representation in the same Period.<br><br>The identifier shall not contain whitespace characters.<br><br>If used in the template-based URL construction as defined in subclause [5.3.9.4.4](05-3-9-4-segment-template.md#c5-3-9-4-4), the string shall only contain characters that are permitted within an HTTP-URL according to IETF RFC 3986. |
| `@bandwidth` | M | Consider a hypothetical constant bitrate channel of bandwidth with the value of this attribute in bits per second (bps). Then, if the Representation is continuously delivered at this bitrate, starting at any SAP that is indicated either by @startWithSAP or by any Segment Index box, a client can be assured of having enough data for continuous playout providing playout begins after @minBufferTime * @bandwidth bits have been received (i.e. at time @minBufferTime after the first bit is received).<br><br>For dependent Representations, this value specifies the bandwidth according to the above definition for the aggregation of this Representation and all complementary Representations.<br><br>For details, see subclause [5.3.5.4](#c5-3-5-4). |
| `@qualityRanking` | O | specifies a quality ranking of the Representation relative to other Representations in the same Adaptation Set. Lower values represent higher quality content. If not present, then no ranking is defined. |
| `@dependencyId` | O | specifies all complementary Representations the Representation depends on in the decoding and/or presentation process as a whitespace-separated list of values of @id attributes.<br><br>If not present, the Representation can be decoded and presented independently of any other Representation.<br><br>This attribute shall not be present where there are no dependencies. |
| `@associationId` | O | specifies all Representations the Representation is associated with in the decoding and/or presentation process as a whitespace-separated list of values of Representation@id attributes. |
| <!--PB printed 53--> `@associationType` | O | specifies, as a whitespace-separated list of values, the kind of association for each Representation the Representation has been associated with through the @associationId attribute. Values taken by this attribute are 4 character codes (4CCs) for track reference types registered in MP4 registration authority.<br><br>This attribute shall not be present when @associationId is not present.<br><br>When present, this attribute shall have as many values as the number of identifiers declared in the @associationId attribute. |
| <!--PB printed 54--> `@mediaStreamStructureId` | O | The attribute may be present for Representations containing video and its semantics are unspecified for any other type of Representations.<br><br>If present, the attribute `@mediaStreamStructureId` specifies a whitespace-separated list of media stream structure identifier values. If media streams share the same media stream structure identifier value, the media streams shall have the following characteristics:<br>— The media streams have the same number of stream access points of type 1 to 3.<br>— The values of T<sub>SAP</sub>, T<sub>DEC</sub>, T<sub>EPT</sub>, and T<sub>PTF</sub> of the *i*-th SAP of type 1 to 3 in one media stream are identical to the values of T<sub>SAP</sub>, T<sub>DEC</sub>, T<sub>EPT</sub>, and T<sub>PTF</sub>, respectively, of the *i*-th SAP of type 1 to 3 in the other media streams for any value of *i* from 1 to the number of SAPs of type 1 to 3 in any of the media streams.<br>— A media stream formed by concatenating the media stream of a first Representation until I<sub>SAU</sub> (exclusive) of the *i*-th SAP of type 1 to 3 and the media stream of a second Representation (having the same media stream structure identifier value as for the first Representation) starting from the I<sub>SAU</sub> (inclusive) of the *i*-th SAP of type 1 to 3 conforms to the specification in which the media stream format is specified for any value of *i* from 1 to the number of SAPs of type 1 to 3 in either media stream. Furthermore, the decoded pictures have an acceptable quality regardless of type of the stream access point access unit used.<br><br>All media stream structure identifier values for one Adaptation Set shall differ from those of another Adaptation Set.<br><br>If not present, then for this Representation no similarities to other Representations are known.<br><br>Indicating multiple media stream structure identifier values for a Representation can be useful in cases where switching between Representations A and B as well as between Representations B and C is allowed at non-IDR intra pictures, but switching between Representations A and C would cause too severe a degradation in the quality of the leading pictures and is hence not allowed. To indicate these permissions and restrictions, Representation A would contain `@mediaStreamStructureId` equal to “1”, Representation B would contain `@mediaStreamStructureId` equal to “1 2”, and Representation C would contain `@mediaStreamStructureId` equal to “2” |
| ***CommonAttributesElements*** | — | Common Attributes and Elements (attributes and elements from base type ***RepresentationBaseType***). For more details, see subclause [5.3.7](05-3-7-common.md#c5-3-7). |
| **BaseURL** | 0…N | specifies a Base URL that can be used for reference resolution and alternative URL selection. For more details, refer to the description in subclause 5.6. |
| **ExtendedBandwidth** | 0…N | specifies an extended bandwidth model with more detailed information on the characteristics of the Representation.<br><br>For more details, see subclause [5.3.5.6](#c5-3-5-6). |
| **SubRepresentation** | 0 … N | specifies information about a Sub-Representation that is embedded in the containing Representation.<br><br>For more details, see subclause [5.3.5.6](#c5-3-5-6). |
| **SegmentBase** | 0...1 | specifies default Segment Base information.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |
| **SegmentList** | 0... 1 | specifies the Segment List information.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |
| **SegmentTemplate** | 0... 1 | specifies the Segment Template information.<br><br>For more details, see subclause [5.3.9](05-3-9-segment-information.md#c5-3-9). |



Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>...<maxOccurs> (N=unbounded)

Elements are bold; attributes are non-bold and preceded with an @; list of elements and attributes is in italics bold referring to those taken from the Base type that has been extended by this type.

<!--PB printed 55-->

<a id="c5-3-5-3"></a>

#### 5.3.5.3 XML syntax

```xml
<xs:complexType name="RepresentationType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Representation</xs:documentation>
    </xs:annotation>
    <xs:complexContent>
        <xs:extension base="RepresentationBaseType">
            <xs:sequence>
                <xs:element name="BaseURL" type="BaseURLType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="ExtendedBandwidth" type="ExtendedBandwidthType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="SubRepresentation" type="SubRepresentationType" minOccurs="0" maxOccurs="unbounded"/>
                <xs:element name="SegmentBase" type="SegmentBaseType" minOccurs="0"/>
                <xs:element name="SegmentList" type="SegmentListType" minOccurs="0"/>
                <xs:element name="SegmentTemplate" type="SegmentTemplateType" minOccurs="0"/>
            </xs:sequence>
            <xs:attribute name="id" type="StringNoWhitespaceType" use="required"/>
            <xs:attribute name="bandwidth" type="xs:unsignedInt" use="required"/>
            <xs:attribute name="qualityRanking" type="xs:unsignedInt"/>
            <xs:attribute name="dependencyId" type="StringVectorType"/>
            <xs:attribute name="associationId" type="StringVectorType"/>
            <xs:attribute name="associationType" type="ListOf4CCType"/>
            <xs:attribute name="mediaStreamStructureId" type="StringVectorType"/>
        </xs:extension>
    </xs:complexContent>
</xs:complexType>
<xs:simpleType name="StringNoWhitespaceType">
    <xs:annotation>
        <xs:documentation xml:lang="en">String without white spaces</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:pattern value="[^\r\n\t \p{Z}]*"/>
    </xs:restriction>
</xs:simpleType>
<xs:simpleType name="StringVectorType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Whitespace-separated list of strings</xs:documentation>
    </xs:annotation>
    <xs:list itemType="xs:string"/>
</xs:simpleType>
<xs:simpleType name="ListOf4CCType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Whitespace separated list of 4CC</xs:documentation>
    </xs:annotation>
    <xs:list itemType="FourCCType"/>
</xs:simpleType>
<xs:simpleType name="FourCCType">
    <xs:annotation>
        <xs:documentation xml:lang="en">4CC as per latest 14496-12</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string"/>
</xs:simpleType>
```

<a id="c5-3-5-4"></a>

#### 5.3.5.4 Relation of Bandwidth and Minimum Buffer Time attributes

The MPD contains a pair of values for a bandwidth and buffering description, namely the Minimum Buffer Time (MBT) expressed by the value of MPD@minBufferTime and bandwidth (BW) expressed by the value of Representation@bandwidth. The following holds:

<!--PB printed 56-->

- The value of the minimum buffer time does not provide any instructions to the client on how long to buffer the media. The value however describes how much buffer a client should have under ideal network conditions. As such, MBT is not describing the burstiness or jitter in the network, it is describing the burstiness or jitter in the content encoding. Together with the BW value, it is a property of the content. Using the "leaky bucket" model, it is the size of the bucket that makes BW true, given the way the content is encoded.
- The minimum buffer time provides information that for each representation, the following shall be true: if the Representation (starting at any segment) is delivered over a constant bitrate channel with bitrate equal to value of the BW attribute then each access unit with presentation time PT is available at the client latest at time with a delay of at most PT + MBT.
- In the absence of any other guidance, the MBT should be set to the maximum GOP size (coded video sequence) of the content, which quite often is identical to the maximum segment duration for the live profile or the maximum subsegment duration for the On-Demand profile. The MBT may be set to a smaller value than maximum (sub)segment duration but should not be set to a higher value.

<a id="c5-3-5-5"></a>

#### 5.3.5.5 Random Access to Representations

Random Access refers to start processing, decoding and presenting the Representation from the random access point at time t onwards by initializing the Representation with the Initialization Segment, if present and decoding and presenting the Representation from the signalled Segment onwards. Random Access point may be signalled with the RandomAccess element as defined in Table 10.

Table 11 provides different random access point types.

<!--PB printed 57-->

Table 10 — Random Access Signalling

| Element or Attribute Name | Use | Description |
|---|---|---|
| **RandomAccess** |  | Random Access Information |
| `@interval` | M | specifies the position of the random access points in the Representations. The information is specified in the scale of the `@timescale` on Representation level. Any Segment for which the MPD start time minus the `@t` value of the `S` element describing the segment is an integer multiple of the product of `@timescale` and `@interval` is a random access opportunity, i.e. it enables randomly access to this Representation with the random access strategy as defined by the `@type` value.<br><br>The value should be chosen such that the resulting time matches MPD start time of segments, otherwise no random access will be described. |
| `@type` | OD<br>default: `"closed"` | specifies the random access strategy for the random access points in by the `@interval` attribute.<br><br>The value shall use a type present in Table 11.<br><br>If the value of the type is unknown, the DASH Client is expected to ignore the containing Random Access element. |
| `@minBufferTime` | O | specifies a common duration used in the definition of the Representation data rate (see `@bandwidth` attribute in subclauses 0 and [5.3.5.4](#c5-3-5-4)).<br><br>If not present, then the value of the MPD level is inherited. |
| `@bandwidth` | O | Consider a hypothetical constant bitrate channel of bandwidth with the value of this attribute in bits per second (bps). Then, if the Representation is continuously delivered at this bitrate, starting at any RAP indicated in this element a client can be assured of having enough data for continuous playout providing playout begins after `@minBufferTime` * `@bandwidth` bits have been received (i.e. at time `@minBufferTime` after the first bit is received).<br><br>For dependent Representations, this value specifies the bandwidth according to the above definition for the aggregation of this Representation and all complementary Representations.<br><br>For details, see subclause [5.3.5.4](#c5-3-5-4).<br><br>If not present, the value of the Representation is inherited. |

<!--PB printed 58-->

Table 11 — Random Access Strategies

| Type | Informative description |
|---|---|
| `closed` | Closed GOP random access. This implies that the segment is a Random Access Segment as well as the segment starts with a SAP type of 1 or 2. SAP type 1 or 2 is a necessary condition, but not sufficient. In addition, all requirements of a Random Access Segment need to be fulfilled. |
| `open` | Open GOP random access. This implies that the segment is a Random Access Segment as well as the segment starts with a SAP type of 1, 2 or 3. SAP type 1, 2 or 3 is a necessary condition, but not sufficient. In addition, all requirements of a Random Access Segment need to be fulfilled. |
| `gradual` | Gradual decoder refresh random access. This implies that the segment is a Random Access Segment as well as the segment starts with a SAP type of 1, 2, 3 or 4. SAP type 1, 2, 3 or 4 is a necessary condition, but not sufficient. In addition, all requirements of a Random Access Segment need to be fulfilled. |

The XML schema snippet is as follows:

```xml
<xs:complexType name="RandomAccessType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Random Access</xs:documentation>
    </xs:annotation>
    <xs:attribute name="interval" type="xs:unsignedInt" use="required"/>
    <xs:attribute name="type" type="RandomAccessTypeType" default="closed"/>
    <xs:attribute name="minBufferTime" type="xs:duration"/>
    <xs:attribute name="bandwidth" type="xs:unsignedInt"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:simpleType name="RandomAccessTypeType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Random Access Type type enumeration</xs:documentation>
    </xs:annotation>
    <xs:restriction base="xs:string">
        <xs:enumeration value="closed"/>
        <xs:enumeration value="open"/>
        <xs:enumeration value="gradual"/>
    </xs:restriction>
</xs:simpleType>
```

<a id="c5-3-5-6"></a>

#### 5.3.5.6 Extended Bandwidth Signalling

Subclause [5.3.5.4](#c5-3-5-4) provides a detailed overview on the usage of @minBufferTime and @bandwidth for determining the bitrate properties of the associated Representation. However, this model does not expose certain extended bandwidth properties of the Representation, for example the nature of the encoding being variable bitrate, or any bitrate properties over different time windows.

For this purpose, each Represention have assigned an extended bandwidth signalling providing additional properties of the Representation. This information is provided as part of the ExtendedBandwidth element as defined in Table 12.

<!--PB printed 59-->

Table 12 — Semantics of Extended Bandwidth signalling

| Element or Attribute Name | Use | Description |
|---|---|---|
| **ExtendedBandwidth** |  |  |
| `@vbr` | OD<br>default=false | If set to true, the content is encoded to primarily have a constant or consistent quality, while at the same time when the signal restrictions on **MPD**`@minBufferTime` and **Representation**`@bandwidth` attributes are reached, the quality of the content may drop. In cases where is cap is not reached, the quality of the content is expected to be consistent.<br><br>If set to false or not present, no information on the nature of the encoding is present.<br><br>NOTE The signalling set to true implies that the actual instantaneous bitrate can frequently be below the maximum. |
| **ModelPair** | 0..N | defines an extended bandwidth model pair that applies for the Representation. |
| `@bufferTime` | M | specifies a duration used in the definition of the Representation data rate together with `@bandwidth` attribute within the same element.<br><br>NOTE There are cases for which a real or expected long-term average bitrate may be usefully signaled. Examples include (i) the signaling of the average bitrate of a Representation in On-Demand case. In this case the @buffer may be set to the duration of the Period and the `@bandwidth` may be set to the size of the Representation. (ii) the signaling of the maximum expected long-term bitrate in Live-Edge Periods: In this case the `@buffer` is set to a large value, for example 1 hour, or 24 hours and the bandwidth expresses the maximum amount of bytes over this period. |
| <!--PB printed 60--> `@bandwidth` | M | Consider a hypothetical constant bitrate channel of bandwidth with the value of this attribute in bits per second (bps). Then, if the Representation is continuously delivered at this bitrate, starting at any signalled SAP of type 1 or 2 included in this Representation element, a client can be assured of having enough data for continuous playout providing playout begins after `@bufferTime` * `@bandwidth` bits have been received (i.e. at time `@bufferTime` after the first bit is received).<br><br>For dependent Representations, this value specifies the bandwidth according to the above definition for the aggregation of this Representation and all complementary Representations.<br><br>For details, see subclause [5.3.5.4](#c5-3-5-4). |

Key

For attributes: M=Mandatory, O=Optional, OD=Optional with Default Value, CM=Conditionally Mandatory

For elements: <minOccurs>..<maxOccurs> (N=unbounded)

Elements are **bold**; attributes are non-bold and preceded with an @.

The XML schema snippet is as follows:

```xml
<xs:complexType name="ExtendedBandwidthType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Extended Bandwidth Model</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:element name="ModelPair" type="ModelPairType" minOccurs="0" maxOccurs="unbounded"/>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="vbr" type="xs:boolean" default="false"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
<xs:complexType name="ModelPairType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Model Pair</xs:documentation>
    </xs:annotation>
    <xs:sequence>
        <xs:any namespace="##other" processContents="lax" minOccurs="0" maxOccurs="unbounded"/>
    </xs:sequence>
    <xs:attribute name="bufferTime" type="xs:duration" use="required"/>
    <xs:attribute name="bandwidth" type="xs:unsignedInt" use="required"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```

<!--PB printed 61-->

## Transcription notes

> **Spec note:** Table 10 `@minBufferTime`: "subclauses 0 and 5.3.5.4" - broken cross-reference ("0"), reproduced as printed.
>
> **Spec note:** 5.3.5.6: "each Represention have assigned" is a typo in the PDF, reproduced as printed. Table 12 `@bufferTime` note says "@buffer" in two places (not `@bufferTime`), as printed.
>
> **Spec note:** Table 9 `@bandwidth` mentions "@startWithSAP" (the attribute is defined in Table 14). As printed.
